use crate::drawing::ToSkia;
use crate::element::{Element, Visual};
use crate::event::Event;
use crate::layout::{BoxConstraints, Geometry};
use crate::text::TextSpan;
use crate::PaintCtx;
use kurbo::{Point, Size};
use skia_safe::textlayout;
use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;
use tracy_client::span;

pub struct Text {
    element: Element,
    text: TextSpan,
    relayout: Cell<bool>,
    paragraph: RefCell<textlayout::Paragraph>,
}

impl Text {
    pub fn new(text: TextSpan) -> Rc<Text> {
        let paragraph = text.build_paragraph();
        Element::new_derived(|element| Text {
            element,
            text,
            relayout: Cell::new(true),
            paragraph: RefCell::new(paragraph),
        })
    }
}

impl Visual for Text {
    fn element(&self) -> &Element {
        &self.element
    }

    fn layout(&self, constraints: &BoxConstraints) -> Geometry {
        // layout paragraph in available space
        let _span = span!("text layout");

        // available space for layout
        let available_width = constraints.max.width;
        let _available_height = constraints.max.height;

        // We can reuse the previous layout if and only if:
        // - the new available width is >= the current paragraph width (otherwise new line breaks are necessary)
        // - the current layout is still valid (i.e. it hasn't been previously invalidated)

        let paragraph = &mut *self.paragraph.borrow_mut();

        if !self.relayout.get() && paragraph.longest_line() <= available_width as f32 {
            let paragraph_size = Size {
                width: paragraph.longest_line() as f64,
                height: paragraph.height() as f64,
            };
            let size = constraints.constrain(paragraph_size);
            return Geometry {
                size,
                baseline: Some(paragraph.alphabetic_baseline() as f64),
                bounding_rect: paragraph_size.to_rect(),
                paint_bounding_rect: paragraph_size.to_rect(),
            };
        }

        paragraph.layout(available_width as skia_safe::scalar);
        let w = paragraph.longest_line() as f64;
        let h = paragraph.height() as f64;
        let alphabetic_baseline = paragraph.alphabetic_baseline();
        let unconstrained_size = Size::new(w, h);
        let size = constraints.constrain(unconstrained_size);
        self.relayout.set(false);

        Geometry {
            size,
            baseline: Some(alphabetic_baseline as f64),
            bounding_rect: size.to_rect(),
            paint_bounding_rect: size.to_rect(),
        }
    }

    fn hit_test(&self, point: Point) -> bool {
        false
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        ctx.with_canvas(|canvas| {
            self.paragraph.borrow().paint(canvas, Point::ZERO.to_skia());
        })
    }

    async fn event(&self, event: &Event)
    where
        Self: Sized,
    {
    }
}
