//! Simple rectangle element

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use kurbo::{Point, Size};
use crate::drawing::Decoration;
use crate::element::{AnyVisual, Element, Visual};
use crate::event::Event;
use crate::layout::{BoxConstraints, Geometry};
use crate::paint_ctx::PaintCtx;

pub struct DecoratedBox<D> {
    element: Element,
    decoration: RefCell<D>,
    size: Cell<Size>,
}

impl<D: Decoration + 'static> DecoratedBox<D> {

    /// Creates a new `DecoratedBox` with the given decoration.
    pub fn new(decoration: D) -> Rc<DecoratedBox<D>> {
        Element::new_derived(|element| DecoratedBox {
            element,
            decoration: RefCell::new(decoration),
            size: Cell::new(Size::new(100.0, 100.0)),
        })
    }

    pub fn set_size(&self, size: Size) {
        self.size.set(size);
    }

    pub fn set_decoration(&self, decoration: D) {
        self.decoration.replace(decoration);
    }
}

impl<D: Decoration + 'static> Visual for DecoratedBox<D> {
    fn element(&self) -> &Element {
        &self.element
    }

    fn layout(&self, children: &[Rc<dyn Visual>], constraints: &BoxConstraints) -> Geometry {
        // stack children
        for child in children {
            child.do_layout(constraints);
        }

        //let mut geometry = self.content.layout(ctx, constraints);
        // assume that the decoration expands the paint bounds
        //geometry.bounding_rect = geometry.bounding_rect.union(geometry.size.to_rect());
        //geometry.paint_bounding_rect = geometry.paint_bounding_rect.union(geometry.size.to_rect());
        //self.size = geometry.size;
        Geometry::new(self.size.get())
    }

    fn hit_test(&self, point: Point) -> bool {
        self.size.get().to_rect().contains(point)
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        ctx.with_canvas(|canvas| {
            self.decoration.borrow().paint(canvas, self.size.get().to_rect());
        });
    }

    async fn event(&self, event: &mut Event)
    where
        Self: Sized
    {}
}