use std::rc::Rc;
use std::sync::Arc;

use kurbo::Vec2;

use crate::{Color, text};
use crate::drawing::BoxShadow;
use crate::element::Visual;
use crate::layout::{Alignment, Sizing};
use crate::layout::flex::{CrossAxisAlignment, MainAxisAlignment};
use crate::style::{Style, StyleExt};
use crate::text::{AttributedStr, TextStyle};
use crate::theme::DARK_THEME;
use crate::widgets::frame::Frame;
use crate::widgets::text::Text;

fn button_style() -> Style {
    thread_local! {
        pub static BUTTON_STYLE: Style = {
            let active = Style::new()
                .background_color(Color::from_hex("4c3e0a"))
                .box_shadows(vec![]);
            let focused = Style::new().border_color(DARK_THEME.accent_color);
            let hovered = Style::new().background_color(Color::from_hex("474029"));
            let mut s = Style::new()
                .background_color(Color::from_hex("211e13"))
                .border_radius(8.0)
                //.width(Sizing::MaxContent)
                //.height(Sizing::MaxContent)
                .min_width(200.0.into())
                .min_height(50.0.into())
                .padding_left(3.0.into())
                .padding_right(3.0.into())
                .padding_top(3.0.into())
                .padding_bottom(3.0.into())
                .border_color(Color::from_hex("4c3e0a"))
                .border_left(1.0.into())
                .border_right(1.0.into())
                .border_top(1.0.into())
                .border_bottom(1.0.into())
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .main_axis_alignment(MainAxisAlignment::Center)
                .box_shadows(vec![
                    /*BoxShadow {
                        color: Color::from_rgb_u8(115, 115, 115),
                        offset: Vec2::new(0.0, 1.0),
                        blur: 0.0,
                        spread: 0.0,
                        inset: true,
                    },*/
                    BoxShadow {
                        color: Color::from_hex("4c3e0a"),
                        offset: Vec2::new(0.0, 1.0),
                        blur: 2.0,
                        spread: -1.0,
                        inset: false,
                    },
                ])
                .active(active)
                .hover(hovered)
                .focus(focused);
            s
        };
    }
    BUTTON_STYLE.with(|s| s.clone())
}

pub fn button(label: impl Into<String>) -> Rc<Frame> {
    let label = label.into();
    let theme = &DARK_THEME;
    let text_style =
        TextStyle::new()
            .font_size(theme.font_size)
            .font_family(theme.font_family)
            .color(Color::from_hex("ffe580"));
    //let text = AttributedStr { str: &label, style:& text_style };
    let text = Text::new(&text!( style(text_style) "{label}" ));
    let mut frame = Frame::new(button_style());
    frame.add_child(&text);
    frame
}
