use crate::application::{spawn, wait_for};
use crate::drawing::{Paint, ToSkia};
use crate::element::{AnyVisual, Element, Visual};
use crate::event::Event;
use crate::handler::Handler;
use crate::layout::{BoxConstraints, Geometry};
use crate::text::{FormattedText, Selection, TextStyle};
use crate::{application, text, Color, PaintCtx};
use futures_util::future::AbortHandle;
use kurbo::{Point, Rect, Size};
use skia_safe::textlayout::{RectHeightStyle, RectWidthStyle};
use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use keyboard_types::Key;
use tracing::warn;
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

#[derive(Debug, Copy, Clone)]
pub enum Movement {
    Left,
    Right,
    LeftWord,
    RightWord,
}

fn prev_grapheme_cluster(text: &str, offset: usize) -> Option<usize> {
    let mut c = GraphemeCursor::new(offset, text.len(), true);
    c.prev_boundary(text, 0).unwrap()
}

fn next_grapheme_cluster(text: &str, offset: usize) -> Option<usize> {
    let mut c = GraphemeCursor::new(offset, text.len(), true);
    c.next_boundary(text, 0).unwrap()
}

struct TextEditState {
    text: String,
    selection: Selection,
    text_style: TextStyle<'static>,
    last_available_width: f64,
    paragraph: skia_safe::textlayout::Paragraph,
    selection_color: Color,
    caret_color: Color,
    relayout: bool,
}

impl TextEditState {
    fn rebuild_paragraph(&mut self) {
        let text = self.text.clone();
        self.paragraph = FormattedText::new(text!( style(self.text_style) "{text}")).inner;
    }
}

/// Single- or multiline text editor.
pub struct TextEdit {
    element: Element,
    selection_changed: Handler<Selection>,
    state: RefCell<TextEditState>,
    in_gesture: Cell<bool>,
    blink_phase: Cell<bool>,
    blink_reset: Cell<bool>,
}

const CARET_BLINK_INITIAL_DELAY: Duration = Duration::from_secs(1);
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);

fn next_word_boundary(text: &str, offset: usize) -> usize {
    let mut pos = offset;
    enum State {
        LeadingWhitespace,
        Alnum,
        NotAlnum,
    }
    let mut state = State::LeadingWhitespace;
    for ch in text[offset..].chars() {
        match state {
            State::LeadingWhitespace => {
                if !ch.is_whitespace() {
                    if ch.is_alphanumeric() {
                        state = State::Alnum;
                    } else {
                        state = State::NotAlnum;
                    }
                }
            }
            State::Alnum => {
                if !ch.is_alphanumeric() {
                    return pos;
                }
            }
            State::NotAlnum => {
                return pos;
            }
        }
        pos += ch.len_utf8();
    }
    pos
}

fn prev_word_boundary(text: &str, offset: usize) -> usize {
    let mut pos = offset;
    enum State {
        LeadingWhitespace,
        Alnum,
        NotAlnum,
    }
    let mut state = State::LeadingWhitespace;
    for ch in text[..offset].chars().rev() {
        match state {
            State::LeadingWhitespace => {
                if !ch.is_whitespace() {
                    if ch.is_alphanumeric() {
                        state = State::Alnum;
                    } else {
                        state = State::NotAlnum;
                    }
                }
            }
            State::Alnum => {
                if !ch.is_alphanumeric() {
                    return pos;
                }
            }
            State::NotAlnum => {
                return pos;
            }
        }
        pos -= ch.len_utf8();
    }
    pos
}

impl TextEdit {
    pub fn new() -> Rc<TextEdit> {
        let text_edit = Element::new_derived(|element| TextEdit {
            element,
            selection_changed: Handler::new(),
            state: RefCell::new(TextEditState {
                text: String::new(),
                selection: Selection::empty(0),
                text_style: TextStyle::default(),
                last_available_width: 0.0,
                paragraph: FormattedText::default().inner,
                selection_color: Color::from_rgba_u8(0, 0, 255, 80),
                caret_color: Color::from_rgba_u8(255, 255, 0, 255),
                relayout: true,
            }),
            in_gesture: Cell::new(false),
            blink_phase: Cell::new(true),
            blink_reset: Cell::new(false),
        });

        // spawn the caret blinker task
        let this_weak = Rc::downgrade(&text_edit);
        spawn(async move {
            'task: loop {
                eprintln!("caret blinker task");
                // Initial delay before blinking
                wait_for(CARET_BLINK_INITIAL_DELAY).await;
                // blinking
                'blink: loop {
                    if let Some(this) = this_weak.upgrade() {
                        if this.blink_reset.replace(false) {
                            // reset requested
                            this.blink_phase.set(true);
                            this.mark_needs_repaint();
                            break 'blink;
                        }
                        this.blink_phase.set(!this.blink_phase.get());
                        this.mark_needs_repaint();
                    } else {
                        // text edit is dead, exit task
                        break 'task;
                    }
                    wait_for(CARET_BLINK_INTERVAL).await;
                }
            }
        });

        text_edit
    }

    /// Resets the phase of the blinking caret.
    pub fn reset_blink(&self) {
        self.blink_phase.set(true);
        self.blink_reset.set(true);
        self.mark_needs_repaint();
    }

    pub fn set_caret_color(&self, color: Color) {
        let this = &mut *self.state.borrow_mut();
        if this.caret_color != color {
            this.caret_color = color;
            self.mark_needs_repaint();
        }
    }

    pub fn set_selection_color(&self, color: Color) {
        let this = &mut *self.state.borrow_mut();
        if this.selection_color != color {
            this.selection_color = color;
            self.mark_needs_repaint();
        }
    }

    pub fn set_text_style(&self, text_style: TextStyle) {
        let this = &mut *self.state.borrow_mut();
        this.text_style = text_style.into_static();
        this.rebuild_paragraph();
        this.relayout = true;
        self.mark_needs_relayout();
    }

    /// Returns the current selection.
    pub fn selection(&self) -> Selection {
        self.state.borrow().selection
    }

    /// Sets the current selection.
    pub fn set_selection(&self, selection: Selection) {
        // TODO clamp selection to text length
        let this = &mut *self.state.borrow_mut();
        if this.selection != selection {
            this.selection = selection;
            self.mark_needs_repaint();
        }
    }

    /// Returns the current text.
    pub fn text(&self) -> String {
        self.state.borrow().text.clone()
    }

    /// Sets the current text.
    pub fn set_text(&self, text: impl Into<String>) {
        // TODO we could compare the previous and new text
        // to relayout only affected lines.
        let this = &mut *self.state.borrow_mut();
        this.text = text.into();
        this.rebuild_paragraph();
        this.relayout = true;
        self.mark_needs_relayout();
    }

    /// NOTE: valid only after first layout.
    pub fn set_cursor_at_point(&self, point: Point, keep_anchor: bool) -> bool {
        // TODO set cursor position based on point
        let this = &mut *self.state.borrow_mut();
        let prev_selection = this.selection;
        let pos = this.paragraph.get_glyph_position_at_coordinate(point.to_skia());
        if keep_anchor {
            this.selection.end = pos.position as usize;
        } else {
            this.selection = Selection::empty(pos.position as usize);
        }
        let selection_changed = this.selection != prev_selection;
        if selection_changed {
            self.mark_needs_repaint();
        }
        selection_changed
    }

    pub fn select_word_under_cursor(&self) {
        let this = &mut *self.state.borrow_mut();
        let text = &this.text;
        let selection = this.selection;
        let range = this.paragraph.get_word_boundary(selection.end as u32);
        this.selection = Selection {
            start: range.start,
            end: range.end,
        };
        self.mark_needs_repaint();
    }

    /// Moves the cursor to the next or previous word boundary.
    pub fn move_cursor_to_next_word(&self, keep_anchor: bool) {
        let this = &mut *self.state.borrow_mut();
        this.selection.end = next_word_boundary(&this.text, this.selection.end);
        if !keep_anchor {
            this.selection.start = this.selection.end;
        }
    }

    pub fn move_cursor_to_prev_word(&self, keep_anchor: bool) {
        let this = &mut *self.state.borrow_mut();
        this.selection.end = prev_word_boundary(&this.text, this.selection.end);
        if !keep_anchor {
            this.selection.start = this.selection.end;
        }
    }

    pub fn move_cursor_to_next_grapheme(&self, keep_anchor: bool) {
        let this = &mut *self.state.borrow_mut();
        this.selection.end = next_grapheme_cluster(&this.text, this.selection.end).unwrap_or(this.selection.end);
        if !keep_anchor {
            this.selection.start = this.selection.end;
        }
    }

    pub fn move_cursor_to_prev_grapheme(&self, keep_anchor: bool) {
        let this = &mut *self.state.borrow_mut();
        this.selection.end = prev_grapheme_cluster(&this.text, this.selection.end).unwrap_or(this.selection.end);
        if !keep_anchor {
            this.selection.start = this.selection.end;
        }
    }

    /// Selects the line under the cursor.
    pub fn select_line_under_cursor(&self) {
        let this = &mut *self.state.borrow_mut();
        let text = &this.text;
        let selection = this.selection;
        let start = text[..selection.end].rfind('\n').map_or(0, |i| i + 1);
        let end = text[selection.end..]
            .find('\n')
            .map_or(text.len(), |i| selection.end + i);
        this.selection = Selection { start, end };
        self.mark_needs_repaint();
    }

    /// Emitted when the selection changes as a result of user interaction.
    pub async fn selection_changed(&self) -> Selection {
        self.selection_changed.wait().await
    }
}

impl Deref for TextEdit {
    type Target = Element;

    fn deref(&self) -> &Self::Target {
        &self.element
    }
}

impl TextEdit {
    /*/// Moves the cursor forward or backward. Returns the new selection.
    fn move_cursor(&self, movement: Movement, modify_selection: bool) -> Selection {
        let text = &*self.text.borrow();
        let selection = self.selection.get();
        let offset = match movement {
            Movement::Left => prev_grapheme_cluster(text, selection.end).unwrap_or(selection.end),
            Movement::Right => next_grapheme_cluster(text, selection.end).unwrap_or(selection.end),
            Movement::LeftWord | Movement::RightWord => {
                // TODO word navigation (unicode word segmentation)
                warn!("word navigation is unimplemented");
                selection.end
            }
        };

        if modify_selection {
            Selection {
                start: selection.start,
                end: offset,
            }
        } else {
            Selection::empty(offset)
        }

    }*/

    /*
        Text representation independent of the editor structure (paragraph).
        Input to text formatter: a list of text runs.
        Formatter: extract formatted lines from the text runs, and provide a mapping from
        visual position to text run index + offset.
        => basically, formatters are **line breakers**
        The editor can then choose to relayout only affected lines.
    */
}

impl Visual for TextEdit {
    fn element(&self) -> &Element {
        &self.element
    }

    fn layout(&self, _children: &[AnyVisual], constraints: &BoxConstraints) -> Geometry {
        let this = &mut *self.state.borrow_mut();

        // determine the available space for layout
        let available_width = constraints.max.width;

        let invalidate_layout = this.relayout || this.last_available_width != available_width;
        if invalidate_layout {
            this.paragraph.layout(available_width as f32);
        }
        this.relayout = false;
        this.last_available_width = available_width;

        let w = this.paragraph.longest_line() as f64;
        let h = this.paragraph.height() as f64;
        let alphabetic_baseline = this.paragraph.alphabetic_baseline();
        let unconstrained_size = Size::new(w, h);
        let size = constraints.constrain(unconstrained_size);

        Geometry {
            size,
            baseline: Some(alphabetic_baseline as f64),
            bounding_rect: size.to_rect(),
            paint_bounding_rect: size.to_rect(),
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let this = &mut *self.state.borrow_mut();
        let bounds = self.geometry().size;

        ctx.with_canvas(|canvas| {
            // draw rect around bounds
            let paint = Paint::from(Color::from_rgba_u8(255, 0, 0, 80)).to_sk_paint(bounds.to_rect());
            canvas.draw_rect(bounds.to_rect().to_skia(), &paint);

            // paint the paragraph
            this.paragraph.paint(canvas, Point::ZERO.to_skia());
            // paint the selection rectangles
            let selection_rects = this.paragraph.get_rects_for_range(
                this.selection.min()..this.selection.max(),
                RectHeightStyle::Tight,
                RectWidthStyle::Tight,
            );
            let selection_paint = Paint::from(this.selection_color).to_sk_paint(bounds.to_rect());
            for text_box in selection_rects {
                canvas.draw_rect(text_box.rect, &selection_paint);
            }

            if self.has_focus() && self.blink_phase.get() {
                if let Some(info) = this.paragraph.get_glyph_cluster_at(this.selection.end) {
                    let caret_rect = Rect::from_origin_size(
                        Point::new((info.bounds.left as f64).round(), (info.bounds.top as f64).round()),
                        Size::new(1.0, info.bounds.height() as f64),
                    );
                    eprintln!("caret_rect: {:?}", caret_rect);
                    let caret_paint = Paint::from(this.caret_color).to_sk_paint(bounds.to_rect());
                    canvas.draw_rect(caret_rect.to_skia(), &caret_paint);
                }
            }

        });
    }

    async fn event(&self, event: &mut Event)
    where
        Self: Sized,
    {
        let mut selection_changed = false;
        match event {
            Event::PointerDown(event) => {
                let pos = event.local_position();
                eprintln!("pointer down point: {:?}", pos);
                if event.repeat_count == 2 {
                    // select word under cursor
                    self.select_word_under_cursor();
                    selection_changed = true;
                } else if event.repeat_count == 3 {
                    // select line under cursor
                } else {
                    selection_changed |= self.set_cursor_at_point(pos, false);
                }
                self.reset_blink();
                self.set_focus();
                self.in_gesture.set(true);
            }
            Event::PointerMove(event) => {
                let pos = event.local_position();
                if self.in_gesture.get() {
                    selection_changed |= self.set_cursor_at_point(pos, true);
                }
                self.reset_blink();
            }
            Event::PointerUp(event) => {
                // TODO set selection based on pointer position
                let pos = event.local_position();
                if self.in_gesture.get() {
                    selection_changed |= self.set_cursor_at_point(pos, true);
                    self.in_gesture.set(false);
                }
            }
            Event::KeyDown(event) => {
                let keep_anchor = event.modifiers.shift();
                let word_nav = event.modifiers.ctrl();
                match event.key {
                    Key::ArrowLeft => {
                        // TODO bidi?
                        if word_nav {
                            self.move_cursor_to_prev_word(keep_anchor);
                        } else {
                            self.move_cursor_to_prev_grapheme(keep_anchor);
                        }
                        selection_changed = true;
                        self.reset_blink();
                    }
                    Key::ArrowRight => {
                        if word_nav {
                            self.move_cursor_to_next_word(keep_anchor);
                        } else {
                            self.move_cursor_to_next_grapheme(keep_anchor);
                        }
                        selection_changed = true;
                        self.reset_blink();
                    }
                    Key::Character(ref s) => {
                        // TODO don't do this, emit the changed text instead
                        let this = &mut *self.state.borrow_mut();
                        let mut text = this.text.clone();
                        let selection = this.selection;
                        text.replace_range(selection.byte_range(), &s);
                        this.text = text;
                        this.rebuild_paragraph();
                        this.relayout = true;
                        this.selection = Selection::empty(selection.min() + s.len());
                        selection_changed = true;
                        self.mark_needs_relayout();
                        self.reset_blink();
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        if selection_changed {
            self.mark_needs_repaint();
            self.selection_changed.emit(self.selection()).await;
        }
    }
}
