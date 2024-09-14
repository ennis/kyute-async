use std::rc::Rc;
use std::sync::Arc;

use kurbo::Vec2;

use crate::Color;
use crate::drawing::BoxShadow;
use crate::element::Visual;
use crate::style::{Style, StyleExt};
use crate::text::{TextSpan, TextStyle};
use crate::theme::DARK_THEME;
use crate::widgets::frame::Frame;
use crate::widgets::text::Text;

fn button_style() -> Style {
    thread_local! {
        pub static BUTTON_STYLE: Style = {
            let active = Style::new()
                .background_color(Color::from_rgb_u8(60, 255, 60))
                .box_shadows(vec![]);
            let focused = Style::new().border_color(DARK_THEME.accent_color);
            //let hovered = Style::new().background_color(Color::from_rgb_u8(100, 100, 100));
            let mut s = Style::new()
                .background_color(Color::from_rgb_u8(88, 88, 88))
                .border_radius(8.0)
                .border_color(Color::from_rgb_u8(49, 49, 49))
                .border_left(1.0.into())
                .border_right(1.0.into())
                .border_top(1.0.into())
                .border_bottom(1.0.into())
                .box_shadows(vec![
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
                        inset: false,
                    },
                ])
                .active(active)
                .focus(focused);
            s
        };
    }
    BUTTON_STYLE.with(|s| s.clone())
}

pub fn button(label: impl Into<String>) -> Rc<Frame> {
    let label = label.into();
    let theme = &DARK_THEME;
    let text_style = Arc::new(
        TextStyle::new()
            .font_size(theme.font_size)
            .font_family(theme.font_family)
            .color(theme.text_color),
    );
    let text = TextSpan::new(label, text_style);
    let label = Text::new(text);
    let mut frame = Frame::new(button_style());
    frame.add_child(&*label);
    frame
}
