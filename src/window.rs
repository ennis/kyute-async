//! Window management.
//!
//! `Window` manages an operating system window that hosts a tree of `Visual` elements.
//! It is responsible for translating window events from winit into `Events` that are dispatched to the `Visual` tree.
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::mem;
use std::rc::Rc;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::Instant;

use futures_util::future::AbortHandle;
use futures_util::StreamExt;
use kurbo::{Affine, Point, Size};
use raw_window_handle::HasWindowHandle;
use skia_safe::font::Edging;
use skia_safe::{Font, FontMgr, FontStyle, Typeface};
use tracing::info;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use winit::platform::windows::WindowBuilderExtWindows;

use crate::app_globals::AppGlobals;
use crate::application::{spawn, with_event_loop_window_target};
use crate::compositor::{ColorType, Layer};
use crate::drawing::ToSkia;
use crate::element::{AnyVisual, HitTestEntry, Visual};
use crate::event::{Event, PointerButton, PointerButtons, PointerEvent};
use crate::handler::Handler;
use crate::layout::BoxConstraints;
use crate::{application, Color, PaintCtx};

/// Stores information about the last click (for double-click handling)
#[derive(Clone, Debug)]
struct LastClick {
    device_id: DeviceId,
    button: PointerButton,
    position: Point,
    time: Instant,
    repeat_count: u32,
}

#[derive(Default)]
struct InputState {
    /// Modifier state. Tracked here because winit doesn't want to give it to us in events.
    modifiers: keyboard_types::Modifiers,
    /// Pointer button state.
    pointer_buttons: PointerButtons,
    last_click: Option<LastClick>,
    /// The widget currently grabbing the pointer.
    pointer_grab: Option<AnyVisual>,
    /// The widget that has the focus for keyboard events.
    focus: Option<AnyVisual>,
    // Result of the previous hit-test
    last_innermost_hit: Option<AnyVisual>,
    last_hits: BTreeSet<AnyVisual>,
    //prev_hit_test_result: Vec<HitTestEntry>,
}

struct WindowInner {
    close_requested: Handler<()>,
    resized: Handler<PhysicalSize<u32>>,
    root: Rc<dyn Visual>,
    layer: Layer,
    window: winit::window::Window,
    hidden_before_first_draw: Cell<bool>,
    cursor_pos: Cell<Point>,
    last_physical_size: Cell<Size>,
    input_state: RefCell<InputState>,
}

impl WindowInner {
    /// Dispatches an event to a target visual in the UI tree.
    ///
    /// It will first invoke the event handler of the target visual.
    /// If the event is "bubbling", it will invoke the event handler of the parent visual,
    /// and so on until the root visual is reached.
    async fn dispatch_event(&self, target: &dyn Visual, mut event: Event, bubbling: bool) {
        // get dispatch chain
        let chain = target.ancestors_and_self();
        assert!(
            chain[0].is_same(&*self.root),
            "target must be a descendant of the root visual"
        );

        // compute local-to-root transforms for each visual in the dispatch chain
        let transforms: Vec<Affine> = chain
            .iter()
            .scan(Affine::default(), |acc, visual| {
                *acc = *acc * visual.transform();
                Some(*acc)
            })
            .collect();

        if bubbling {
            // dispatch the event, bubbling from the target up the root
            for (visual, transform) in chain.iter().rev().zip(transforms.iter().rev()) {
                event.set_transform(transform);
                visual.send_event(&mut event).await;
            }
        } else {
            // dispatch the event to the target only
            event.set_transform(transforms.last().unwrap());
            target.send_event(&mut event).await;
        }
    }

    /// Dispatches a pointer event in the UI tree.
    ///
    /// It first determines the target of the event (i.e. either the pointer-capturing element or
    /// the deepest element that passes the hit-test), then propagates the event to the target with `send_event`.
    ///
    /// TODO It should also handle focus and hover update events (FocusGained/Lost, PointerOver/Out).
    ///
    /// # Return value
    ///
    /// Returns true if the app logic should re-run in response of the event.
    async fn dispatch_pointer_event(
        &self,
        event: Event,
        hit_position: Point,
        //time: Duration,
    ) {
        let mut input_state = self.input_state.borrow_mut();

        let hits = self.root.do_hit_test(hit_position);
        let innermost_hit = hits.last().cloned();

        // If something is grabbing the pointer, then the event is delivered to that element;
        // otherwise it is delivered to the innermost widget that passes the hit-test.
        let target = input_state.pointer_grab.take().or(innermost_hit.clone());

        if let Some(target) = target {
            self.dispatch_event(&*target, event, true).await;
        }

        let p = PointerEvent {
            position: hit_position,
            modifiers: input_state.modifiers,
            buttons: input_state.pointer_buttons,
            button: None,
            repeat_count: 0,
            transform: Default::default(),
            request_capture: false,
        };

        // convert hits to set
        let hits_set = BTreeSet::from_iter(hits);

        let hit_changed = input_state.last_innermost_hit != innermost_hit;

        // send pointerout
        if hit_changed {
            if let Some(ref out) = input_state.last_innermost_hit {
                self.dispatch_event(&**out, Event::PointerOut(p), true).await;
            }
        }
        // send pointerleave
        let leaving = input_state.last_hits.difference(&hits_set);
        for v in leaving {
            self.dispatch_event(&**v, Event::PointerLeave(p), false).await;
        }

        // send pointerover
        if hit_changed {
            if let Some(ref over) = innermost_hit {
                self.dispatch_event(&**over, Event::PointerOver(p), true).await;
            }
        }

        // send pointerenter
        let entering = hits_set.difference(&input_state.last_hits);
        for v in entering {
            self.dispatch_event(&**v, Event::PointerEnter(p), false).await;
        }

        // update last hits
        input_state.last_hits = hits_set;
        input_state.last_innermost_hit = innermost_hit;
    }

    /// Converts a winit mouse event to an Event, and update internal state.
    fn convert_mouse_event(&self, device_id: DeviceId, button: MouseButton, state: ElementState) -> Option<Event> {
        let mut input_state = self.input_state.borrow_mut();
        let button = match button {
            MouseButton::Left => PointerButton::LEFT,
            MouseButton::Right => PointerButton::RIGHT,
            MouseButton::Middle => PointerButton::MIDDLE,
            MouseButton::Back => PointerButton::X1,
            MouseButton::Forward => PointerButton::X2,
            MouseButton::Other(_) => {
                // FIXME ignore extended buttons for now, but they should really be propagated as well
                return None;
            }
        };
        // update tracked state
        if state.is_pressed() {
            input_state.pointer_buttons.set(button);
        } else {
            input_state.pointer_buttons.reset(button);
        }
        let click_time = Instant::now();

        /*// implicit pointer ungrab
        if !state.is_pressed() {
            self.input_state.pointer_grab = None;
        }*/

        // determine the repeat count (double-click, triple-click, etc.) for button down event
        let repeat_count = match &mut input_state.last_click {
            Some(ref mut last)
                if last.device_id == device_id
                    && last.button == button
                    && last.position == self.cursor_pos.get()
                    && (click_time - last.time) < AppGlobals::get().double_click_time() =>
            {
                // same device, button, position, and within the platform specified double-click time
                if state.is_pressed() {
                    last.repeat_count += 1;
                    last.repeat_count
                } else {
                    // no repeat for release events (although that could be possible?)
                    1
                }
            }
            other => {
                // no match, reset
                if state.is_pressed() {
                    *other = Some(LastClick {
                        device_id,
                        button,
                        position: self.cursor_pos.get(),
                        time: click_time,
                        repeat_count: 1,
                    });
                } else {
                    *other = None;
                };
                1
            }
        };
        let pe = PointerEvent {
            position: self.cursor_pos.get(),
            modifiers: input_state.modifiers,
            buttons: input_state.pointer_buttons,
            button: Some(button),
            repeat_count: repeat_count as u8,
            transform: Default::default(),
            request_capture: false,
        };

        let event = if state.is_pressed() {
            Event::PointerDown(pe)
        } else {
            Event::PointerUp(pe)
        };

        Some(event)
    }

    /// Converts & dispatches a winit window event.
    async fn dispatch_winit_input_event(&self, event: &WindowEvent) {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos.set(Point::new(position.x, position.y));
                self.window.request_redraw();
            }
            WindowEvent::Touch(touch) => {
                self.cursor_pos.set(Point::new(touch.location.x, touch.location.y));
                self.window.request_redraw();
            }
            WindowEvent::MouseInput {
                button,
                state,
                device_id,
            } => {
                if let Some(event) = self.convert_mouse_event(*device_id, *button, *state) {
                    self.dispatch_pointer_event(event, self.cursor_pos.get()).await;
                }
            }
            WindowEvent::CloseRequested => {
                self.close_requested.emit(()).await;
            }
            WindowEvent::Resized(size) => {
                self.resized.emit(*size).await;
                if size.width != 0 && size.height != 0 {
                    // resize the compositor layer
                    let size = Size::new(size.width as f64, size.height as f64);
                    self.layer.set_surface_size(size);
                }
            }
            WindowEvent::RedrawRequested => {
                self.do_redraw();
            }
            event => {
                let dummy_pointer_event = PointerEvent {
                    position: kurbo::Point::new(0.0, 0.0),
                    modifiers: Default::default(),
                    buttons: Default::default(),
                    button: None,
                    repeat_count: 0,
                    transform: Default::default(),
                    request_capture: false,
                };
                //eprintln!("before events");
                //this.root.send_event(&Event::PointerDown(dummy_pointer_event)).await;
                //this.root.send_event(&Event::PointerUp(dummy_pointer_event)).await;
                //eprintln!("after events");
            }
        }

        if self.root.needs_repaint() {
            self.window.request_redraw();
        }
    }

    fn do_redraw(&self) {
        let scale_factor = self.window.scale_factor();
        let physical_size = self.window.inner_size();
        if physical_size.width == 0 || physical_size.height == 0 {
            return;
        }
        let size = physical_size.to_logical(scale_factor);
        let physical_size = Size::new(physical_size.width as f64, physical_size.height as f64);
        let size = Size::new(size.width, size.height);

        if physical_size != self.last_physical_size.get() {
            self.last_physical_size.set(physical_size);
            //self.layer.set_surface_size(physical_size);
        }

        if self.root.needs_relayout() {
            let _geom = self.root.do_layout(&BoxConstraints::loose(size));
        }

        let surface = self.layer.acquire_drawing_surface();

        // FIXME: only clear and flip invalid regions
        {
            let mut skia_surface = surface.surface();
            skia_surface.canvas().clear(Color::from_hex("#151515").to_skia());
            draw_crosshair(skia_surface.canvas(), self.cursor_pos.get());

            // draw a text blob in the middle of the window
            let mut paint = skia_safe::Paint::default();
            paint.set_color(skia_safe::Color::WHITE);
            paint.set_anti_alias(true);
            paint.set_stroke_width(1.0);
            paint.set_style(skia_safe::paint::Style::Fill);
            let mut font = Font::from_typeface(default_typeface(), 12.0);
            font.set_subpixel(true);
            font.set_edging(Edging::SubpixelAntiAlias);
            let text_blob = skia_safe::TextBlob::from_str("Hello, world!", &font).unwrap();
            skia_surface.canvas().draw_text_blob(
                text_blob,
                (size.width as f32 / 2.0, size.height as f32 / 2.0),
                &paint,
            );

            self.root.do_paint(&surface, scale_factor);
        }

        /*// Now paint the UI tree.
        {
            let mut paint_ctx = PaintCtx {
                scale_factor: this.window.scale_factor(),
                window_transform: Default::default(),
                surface: &surface,
                //debug_info: Default::default(),
            };
            // TODO
            this.root.paint(&mut paint_ctx);

            /*// Paint the debug overlay if there's one.
            if let Some(ref debug_overlay) = options.debug_overlay {
                debug_overlay.paint(&mut paint_ctx);
            }*/

            // Save debug information after painting.
            //self.paint_debug_info.replace(paint_ctx.debug_info);
        }*/

        // Nothing more to paint, release the surface.
        //
        // This flushes the skia command buffers, and presents the surface to the compositor.
        drop(surface);

        // Windows are initially created hidden, and are only shown after the first frame is painted.
        // Now that we've rendered the first frame, we can reveal it.
        if self.hidden_before_first_draw.get() {
            self.hidden_before_first_draw.set(false);
            self.window.set_visible(true);
        }

        //self.clear_change_flags(ChangeFlags::PAINT);

        // Wait for the compositor to be ready to render another frame (this is to reduce latency)
        // FIXME: this assumes that there aren't any other windows waiting to be painted!
        self.layer.wait_for_presentation();

        sleep(std::time::Duration::from_millis(5));
    }
}

pub struct Window {
    shared: Rc<WindowInner>,
    /// Handle to the event dispatcher task.
    dispatcher: AbortHandle,
}

fn draw_crosshair(canvas: &skia_safe::Canvas, pos: Point) {
    let mut paint = skia_safe::Paint::default();
    paint.set_color(skia_safe::Color::WHITE);
    paint.set_anti_alias(true);
    paint.set_stroke_width(1.0);
    paint.set_style(skia_safe::paint::Style::Stroke);

    let x = pos.x as f32 + 0.5;
    let y = pos.y as f32 + 0.5;
    canvas.draw_line((x - 20.0, y), (x + 20.0, y), &paint);
    canvas.draw_line((x, y - 20.0), (x, y + 20.0), &paint);
    // draw a circle around the crosshair
    canvas.draw_circle((x, y), 10.0, &paint);
}

static DEFAULT_TYPEFACE: OnceLock<Typeface> = OnceLock::new();

pub fn default_typeface() -> Typeface {
    DEFAULT_TYPEFACE
        .get_or_init(|| {
            let font_mgr = FontMgr::new();
            font_mgr.match_family_style("Inter", FontStyle::default()).unwrap()
        })
        .clone()
}

impl Window {
    /// TODO builder
    pub fn new(title: &str, size: kurbo::Size, root: &dyn Visual) -> Self {
        let window = with_event_loop_window_target(|event_loop| {
            // the window is initially invisible, we show it after the first frame is painted.
            winit::window::WindowBuilder::new()
                .with_title(title)
                .with_no_redirection_bitmap(true)
                .with_blur(true)
                //.with_visible(false)
                .with_inner_size(winit::dpi::LogicalSize::new(size.width, size.height))
                .build(&event_loop)
                .unwrap()
        });

        // Setup compositor layer
        // Get the physical size from the window
        let phy_size = window.inner_size();
        let phy_size = Size::new(phy_size.width as f64, phy_size.height as f64);
        let layer = Layer::new_surface(phy_size, ColorType::RGBAF16);

        let raw_window_handle = window
            .window_handle()
            .expect("failed to get raw window handle")
            .as_raw();
        unsafe {
            // Bind the layer to the window
            // SAFETY: idk? the window handle is valid?
            layer.bind_to_window(raw_window_handle);
        }

        // On windows, the initial wait is important:
        // see https://learn.microsoft.com/en-us/windows/uwp/gaming/reduce-latency-with-dxgi-1-3-swap-chains#step-4-wait-before-rendering-each-frame
        layer.wait_for_presentation();

        // FIXME: we don't really need a channel, we could just register a trait object like we do
        // with Visuals
        let mut input_events = application::register_window(window.id());
        let shared = Rc::new(WindowInner {
            close_requested: Handler::new(),
            resized: Handler::new(),
            root: root.rc(),
            layer,
            window,
            hidden_before_first_draw: Cell::new(true),
            cursor_pos: Cell::new(Default::default()),
            last_physical_size: Cell::new(phy_size),
            input_state: Default::default(),
        });

        // spawn a task that dispatches window events to events on individual elements
        let dispatcher = {
            let this = shared.clone();
            spawn(async move {
                while let Some(input_event) = input_events.next().await {
                    // eprintln!("input_event: {:?}", input_event);
                    this.dispatch_winit_input_event(&input_event).await;
                }
            })
        };

        Window { shared, dispatcher }
    }

    /// Waits for the window to be closed.
    pub async fn close_requested(&self) {
        self.shared.close_requested.wait().await
    }

    /// Waits for the window to be resized.
    pub async fn resized(&self) -> PhysicalSize<u32> {
        self.shared.resized.wait().await
    }

    /// Hides the window.
    pub fn hide(&self) {
        self.shared.window.set_visible(false);
    }

    pub fn is_hidden(&self) -> bool {
        !self.shared.window.is_visible().unwrap()
    }
}
