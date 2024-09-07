//! Padding visual


use kurbo::{Insets, Point};
use crate::element::{Element, VisualDelegate};
use crate::layout::{BoxConstraints, Geometry};

pub struct Padding {
    pub padding: Insets,
}

impl VisualDelegate for Padding {
    fn layout(&self, this_element: &Element, children: &[Element], box_constraints: BoxConstraints) -> Geometry {
        todo!()
    }

    fn hit_test(&self, this_element: &Element, point: Point) -> Option<Element> {
        todo!()
    }
}