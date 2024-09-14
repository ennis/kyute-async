#![feature(type_alias_impl_trait)]
#![feature(async_fn_track_caller)]
#![feature(const_fn_floating_point_arithmetic)]

pub use kurbo::{self, Size};
pub use skia_safe as skia;
use tokio::select;

pub use color::Color;
pub use paint_ctx::PaintCtx;

// kurbo reexports
use crate::window::Window;

mod app_globals;
mod application;
mod backend;
pub mod color;
mod compositor;
mod drawing;
mod element;
mod event;
mod handler;
mod paint_ctx;
mod reactive;
mod skia_backend;
mod text;
mod widgets;
mod window;
pub mod theme;
mod style;
pub mod layout;
//mod element;

use widgets::button::button;

fn main() {
    application::run(async {
       
        let button = button("Click me!");
        let mut main_window = Window::new("Hello, world!", Size::new(800.0, 600.0), &*button);

        loop {
            select! {
                /*_ = button.clicked() => {
                    eprintln!("Button clicked");
                }*/
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
