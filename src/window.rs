use crate::application;
use crate::application::{spawn, with_event_loop_window_target};
use crate::element::Element;
use crate::handler::Handler;
use async_stream::stream;
use futures_util::future::AbortHandle;
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use std::cell::RefCell;
use std::rc::Rc;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;

#[derive(Default)]
struct WindowEvents {
    close_requested: Handler<()>,
    resized: Handler<PhysicalSize<u32>>,
}

pub struct Window {
    window: winit::window::Window,
    /// Root element of the window.
    root: RefCell<Element>,
    /// Event handlers.
    events: Rc<WindowEvents>,
    /// Handle to the event dispatcher task.
    dispatcher: AbortHandle,
}

impl Window {
    /// TODO builder
    pub fn new(title: &str, size: kurbo::Size) -> Self {
        let window = with_event_loop_window_target(|event_loop| {
            winit::window::WindowBuilder::new()
                .with_title(title)
                .with_inner_size(winit::dpi::LogicalSize::new(size.width, size.height))
                .build(&event_loop)
                .unwrap()
        });

        let mut input_events = application::register_window(window.id());
        let events = Rc::new(WindowEvents::default());

        // spawn a task that dispatches window events to events on individual elements
        let dispatcher = {
            let events = events.clone();
            spawn(async move {
                while let Some(input_event) = input_events.next().await {
                    match input_event {
                        WindowEvent::CloseRequested => {
                            events.close_requested.emit(()).await;
                        }
                        WindowEvent::Resized(size) => {
                            events.resized.emit(size).await;
                            // TODO relayout
                        }
                        event => {
                            // TODO: figure out which element the event should be forwarded to
                            // forward the event to the root element
                        }
                    }
                }
            })
        };

        Window {
            window,
            root: RefCell::new(Element::<()>::new().into()),
            events,
            dispatcher,
        }
    }

    /// Sets the root element of the window.
    pub fn set_contents(&self, element: Element) {
        self.root.replace(element);
    }

    /// Waits for the window to be closed.
    pub async fn close_requested(&self) {
        self.events.close_requested.wait().await
    }

    /// Waits for the window to be resized.
    pub async fn resized(&self) -> PhysicalSize<u32> {
        self.events.resized.wait().await
    }

    /// Hides the window.
    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn is_hidden(&self) -> bool {
        !self.window.is_visible().unwrap()
    }
}
