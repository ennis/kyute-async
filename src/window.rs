use std::cell::Cell;
use std::rc::Rc;

use futures_util::future::AbortHandle;
use futures_util::StreamExt;
use kurbo::{Point, Size};
use raw_window_handle::HasWindowHandle;
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
use crate::{application, Color, PaintCtx};

struct WindowInner {
    close_requested: Handler<()>,
    resized: Handler<PhysicalSize<u32>>,
    root: Rc<dyn Visual>,
    layer: Layer,
    window: winit::window::Window,
    hidden_before_first_draw: Cell<bool>,
    cursor_pos: Cell<Point>,
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

impl Window {
    /// TODO builder
    pub fn new(title: &str, size: kurbo::Size, root: &dyn Visual) -> Self {
        let window = with_event_loop_window_target(|event_loop| {
            // the window is initially invisible, we show it after the first frame is painted.
            winit::window::WindowBuilder::new()
                .with_title(title)
                .with_no_redirection_bitmap(true)
                //.with_visible(false)
                .with_inner_size(winit::dpi::LogicalSize::new(size.width, size.height))
                .build(&event_loop)
                .unwrap()
        });

        // Setup compositor layer
        let size = window.inner_size();
        let layer = Layer::new_surface(Size::new(size.width as f64, size.height as f64), ColorType::RGBAF16);

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
        });

        // spawn a task that dispatches window events to events on individual elements
        let dispatcher = {
            let this = shared.clone();
            spawn(async move {
                while let Some(input_event) = input_events.next().await {
                    //eprintln!("input_event: {:?}", input_event);
                    match input_event {
                        WindowEvent::CursorMoved {position, ..} => {
                            this.cursor_pos.set(Point::new(position.x as f64, position.y as f64));
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
                            this.window.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            eprintln!("RedrawRequested");
                            // Acquire a drawing surface and clear it.
                            let surface =  this.layer.acquire_drawing_surface();

                            // FIXME: only clear and flip invalid regions
                            {
                                let mut skia_surface = surface.surface();
                                skia_surface.canvas().clear(Color::from_hex("#111155").to_skia());
                                draw_crosshair(skia_surface.canvas(), this.cursor_pos.get());
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
                            if this.hidden_before_first_draw.get() {
                                this.hidden_before_first_draw.set(false);
                                this.window.set_visible(true);
                            }

                            //self.clear_change_flags(ChangeFlags::PAINT);

                            // Wait for the compositor to be ready to render another frame (this is to reduce latency)
                            // FIXME: this assumes that there aren't any other windows waiting to be painted!
                            this.layer.wait_for_presentation();
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

        Window {
            shared,
            dispatcher,
        }
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
