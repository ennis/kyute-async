#![feature(type_alias_impl_trait)]

use tokio::select;
pub use kurbo::{self, Size};

// kurbo reexports
use crate::window::Window;

//mod backend;
//mod skia_backend;
//mod app_globals;
//mod compositor;
mod application;
mod element;
mod event;
mod handler;
mod layout;
mod reactive;
mod widgets;
mod window;
//mod element;

fn main() {
    application::run(async {
        let mut main_window = Window::new("Hello, world!", Size::new(800.0, 600.0));

        loop {
            select! {
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
