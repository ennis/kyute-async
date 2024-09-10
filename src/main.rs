#![feature(type_alias_impl_trait)]
#![feature(async_fn_track_caller)]
#![feature(const_fn_floating_point_arithmetic)]

use crate::widgets::button::Button;
pub use kurbo::{self, Size};
use kurbo::{Insets, Vec2};
use std::sync::Arc;
use tokio::select;

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
mod layout;
mod paint_ctx;
mod reactive;
mod skia_backend;
mod text;
mod widgets;
mod window;
//mod element;

use crate::drawing::{BorderStyle, BoxShadow, RoundedRectBorder, ShapeDecoration};
use crate::text::{TextSpan, TextStyle};
use crate::widgets::decorated_box::DecoratedBox;
use crate::widgets::text::Text;
pub use color::Color;
pub use paint_ctx::PaintCtx;
pub use skia_safe as skia;
use smallvec::smallvec;

fn main() {
    application::run(async {
        let text_style = Arc::new(
            TextStyle::new()
                .font_size(12.0)
                .font_weight(500)
                .font_family("Inter")
                .color(Color::from_rgb_u8(0, 0, 0)),
        );
        let text = TextSpan::new(
            "Hello, world! Lorem \
        ipsum dolrokcxvjvdofsmwijh odsfrgij opfdishg psdfuioghfdspiuo
        "
            .to_string(),
            text_style,
        );
        let text = Text::new(text);

        let decobox = DecoratedBox::new(ShapeDecoration {
            fill: Color::from_rgb_u8(100, 100, 100).into(),
            border: RoundedRectBorder {
                color: Color::from_rgb_u8(49, 49, 49),
                radius: 8.0,
                dimensions: Insets::uniform(1.0),
                style: BorderStyle::Solid,
            },
            shadows: smallvec![
                BoxShadow {
                    color: Color::from_rgb_u8(115, 115, 115),
                    offset: Vec2::new(0.0, 1.0),
                    blur: 0.0,
                    spread: 0.0,
                    inset: true,
                },
                BoxShadow {
                    color: Color::from_rgb_u8(49, 49, 49),
                    offset: Vec2::new(0.0, 1.0),
                    blur: 2.0,
                    spread: -1.0,
                    inset: false
                }
            ],
        });

        let mut main_window = Window::new("Hello, world!", Size::new(800.0, 600.0), &*decobox);

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
