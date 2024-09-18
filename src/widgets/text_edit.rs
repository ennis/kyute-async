use crate::drawing::{Paint, ToSkia};
use crate::element::{AnyVisual, Element, Visual};
use crate::event::Event;
use crate::handler::Handler;
use crate::layout::{BoxConstraints, Geometry, IntrinsicSizes};
use crate::text::{FormattedText, Selection, TextStyle};
use crate::{Color, PaintCtx, text};
use kurbo::{Point, Size};
use skia_safe::textlayout::{RectHeightStyle, RectWidthStyle};
use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell};
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use tracing::warn;
use unicode_segmentation::GraphemeCursor;

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
    relayout: bool,
}

impl TextEditState {
    fn rebuild_paragraph(&mut self) {
        let text = self.text.clone();
        self.paragraph = FormattedText::new(text!( style(self.text_style) "{text}")).inner;;
    }
}

/// Single- or multiline text editor.
pub struct TextEdit {
    element: Element,
    selection_changed: Handler<Selection>,
    state: RefCell<TextEditState>,
    in_gesture: Cell<bool>,
}

impl TextEdit {
    pub fn new() -> Rc<TextEdit> {
        Element::new_derived(|element| TextEdit {
            element,
            selection_changed: Handler::new(),
            state: RefCell::new(TextEditState {
                text: String::new(),
                selection: Selection::empty(0),
                text_style: TextStyle::default(),
                last_available_width: 0.0,
                paragraph: FormattedText::default().inner,
                selection_color: Color::from_rgba_u8(0, 0, 255, 80),
                relayout: true,
            }),
            in_gesture: Cell::new(false),
        })
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

// Strategy: editing buffer
// - one vec per line

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

            //this.paragraph.

            // draw cursor
            //let cursor_pos = this.paragraph.get
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
                self.in_gesture.set(true);
            }
            Event::PointerMove(event) => {
                let pos = event.local_position();
                if self.in_gesture.get() {
                    selection_changed |= self.set_cursor_at_point(pos, true);
                }
            }
            Event::PointerUp(event) => {
                // TODO set selection based on pointer position
                let pos = event.local_position();
                if self.in_gesture.get() {
                    selection_changed |= self.set_cursor_at_point(pos, true);
                    self.in_gesture.set(false);
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
