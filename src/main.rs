#![feature(type_alias_impl_trait)]
#![feature(async_fn_track_caller)]
#![feature(const_fn_floating_point_arithmetic)]

use tokio::select;
pub use kurbo::{self, Size};
use crate::widgets::button::Button;

// kurbo reexports
use crate::window::Window;

mod backend;
mod skia_backend;
mod compositor;
mod application;
mod element;
mod event;
mod handler;
mod layout;
mod reactive;
mod widgets;
mod window;
mod drawing;
pub mod color;
mod app_globals;
mod paint_ctx;
//mod element;

pub use color::Color;
pub use skia_safe as skia;
pub use paint_ctx::PaintCtx;

fn main() {
    application::run(async {

        let button = Button::new();
        let mut main_window = Window::new("Hello, world!", Size::new(800.0, 600.0), &*button);

        loop {
            select! {
                _ = button.clicked() => {
                    eprintln!("Button clicked");
                }
                _ = main_window.close_requested() => {
                    eprintln!("Window closed");
                    break
                }
                size = main_window.resized() => {
                    eprintln!("Window resized to {:?}", size);
                }
            }
        }

        application::quit();
    })
    .unwrap()
}
