use std::cell::{Cell, OnceCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::thread::sleep;

use futures_util::future::AbortHandle;
use futures_util::StreamExt;
use kurbo::{Point, Size};
use raw_window_handle::HasWindowHandle;
use skia_safe::font::Edging;
use skia_safe::utils::text_utils::Align;
use skia_safe::{Font, FontMgr, FontStyle, Typeface};
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::platform::windows::WindowBuilderExtWindows;

use crate::app_globals::AppGlobals;
use crate::application::{spawn, with_event_loop_window_target};
use crate::compositor::{ColorType, Layer};
use crate::drawing::ToSkia;
use crate::element::Visual;
use crate::event::{Event, PointerEvent};
use crate::handler::Handler;
use crate::layout::BoxConstraints;
use crate::{application, Color, PaintCtx};

struct WindowInner {
    close_requested: Handler<()>,
    resized: Handler<PhysicalSize<u32>>,
    root: Rc<dyn Visual>,
    layer: Layer,
    window: winit::window::Window,
    hidden_before_first_draw: Cell<bool>,
    cursor_pos: Cell<Point>,
    last_physical_size: Cell<Size>,
}

impl WindowInner {
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

        let _geom = self
            .root
            .layout(&BoxConstraints::loose(size));

        let surface = self.layer.acquire_drawing_surface();

        // FIXME: only clear and flip invalid regions
        {
            let mut skia_surface = surface.surface();
            skia_surface.canvas().clear(Color::from_hex("#151515").to_skia());
            draw_crosshair(skia_surface.canvas(), self.cursor_pos.get());

            // draw a text blob in the middle of the window
            let mut paint = skia_safe::Paint::default();
            paint.set_color(skia_safe::Color::BLACK);
            paint.set_anti_alias(true);
            paint.set_stroke_width(1.0);
            paint.set_style(skia_safe::paint::Style::Fill);
            let mut font = Font::from_typeface(default_typeface(), 14.0);
            font.set_subpixel(true);
            font.set_edging(Edging::SubpixelAntiAlias);
            let text_blob = skia_safe::TextBlob::from_str("Hello, world!", &font).unwrap();
            skia_surface.canvas().draw_text_blob(
                text_blob,
                (size.width as f32 / 2.0, size.height as f32 / 2.0),
                &paint,
            );

            let mut paint_ctx = PaintCtx {
                scale_factor,
                window_transform: Default::default(),
                surface: &surface,
            };

            self.root.paint(&mut paint_ctx);
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

       // sleep(std::time::Duration::from_millis(13));
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
        });

        // spawn a task that dispatches window events to events on individual elements
        let dispatcher = {
            let this = shared.clone();
            spawn(async move {
                while let Some(input_event) = input_events.next().await {
                    // eprintln!("input_event: {:?}", input_event);
                    match input_event {
                        WindowEvent::CursorMoved { position, .. } => {
                            this.cursor_pos.set(Point::new(position.x as f64, position.y as f64));
                            this.window.request_redraw();
                        }
                        WindowEvent::Touch(touch) => {
                            this.cursor_pos
                                .set(Point::new(touch.location.x as f64, touch.location.y as f64));
                            this.window.request_redraw();
                        }
                        WindowEvent::CloseRequested => {
                            this.close_requested.emit(()).await;
                        }
                        WindowEvent::Resized(size) => {
                            this.resized.emit(size).await;
                            if size.width != 0 && size.height != 0 {
                                // resize the compositor layer
                                let size = Size::new(size.width as f64, size.height as f64);
                                this.layer.set_surface_size(size);
                            }
                            // redraw now
                            //this.do_redraw();
                            //this.window.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            this.do_redraw();
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
