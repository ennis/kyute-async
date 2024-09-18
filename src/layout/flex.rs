use kurbo::{Point, Rect, Size, Vec2};
use std::rc::Rc;

use crate::element::{AnyVisual, AttachedProperty, Element, Visual};
use crate::event::Event;
use crate::layout::{BoxConstraints, Geometry};
use crate::PaintCtx;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Default)]
pub enum MainAxisAlignment {
    #[default]
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Default)]
pub enum CrossAxisAlignment {
    #[default]
    Start,
    End,
    Center,
    Stretch,
    Baseline,
}

pub struct FlexFactor;

impl AttachedProperty for FlexFactor {
    type Value = f64;
}

/*
////////////////////////////////////////////////////////////////////////////////////////////////////
pub struct Flex {
    pub element: Element,
    pub axis: Axis,
    pub main_axis_alignment: MainAxisAlignment,
    pub cross_axis_alignment: CrossAxisAlignment,
}

impl Flex {
    pub fn new(axis: Axis) -> Rc<Flex> {
        Element::new_derived(|element| Flex {
            element,
            axis,
            main_axis_alignment: MainAxisAlignment::Start,
            cross_axis_alignment: CrossAxisAlignment::Start,
        })
    }

    pub fn row() -> Rc<Flex> {
        Flex::new(Axis::Horizontal)
    }

    pub fn column() -> Rc<Flex> {
        Flex::new(Axis::Vertical)
    }

    pub fn push(&self, item: &dyn Visual) {
        // FIXME yeah that's not very good looking
        (self as &dyn Visual).add_child(item);
    }

    pub fn push_flex(&self, item: &dyn Visual, flex: f64) {
        FlexFactor.set(item, flex);
        (self as &dyn Visual).add_child(item);
    }
}
*/

////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Axis {
    #[default]
    Vertical,
    Horizontal,
}

impl Axis {
    fn constraints(
        &self,
        main_axis_min: f64,
        main_axis_max: f64,
        cross_axis_min: f64,
        cross_axis_max: f64,
    ) -> BoxConstraints {
        match self {
            Axis::Horizontal => BoxConstraints {
                min: Size {
                    width: main_axis_min,
                    height: cross_axis_min,
                },
                max: Size {
                    width: main_axis_max,
                    height: cross_axis_max,
                },
            },
            Axis::Vertical => BoxConstraints {
                min: Size {
                    width: cross_axis_min,
                    height: main_axis_min,
                },
                max: Size {
                    width: cross_axis_max,
                    height: main_axis_max,
                },
            },
        }
    }
}

/// Helper trait for main axis/cross axis sizes
trait AxisSizeHelper {
    fn main_length(&self, main_axis: Axis) -> f64;
    fn cross_length(&self, main_axis: Axis) -> f64;

    fn from_main_cross(main_axis: Axis, main: f64, cross: f64) -> Self;
}

impl AxisSizeHelper for Size {
    fn main_length(&self, main_axis: Axis) -> f64 {
        match main_axis {
            Axis::Horizontal => self.width,
            Axis::Vertical => self.height,
        }
    }

    fn cross_length(&self, main_axis: Axis) -> f64 {
        match main_axis {
            Axis::Horizontal => self.height,
            Axis::Vertical => self.width,
        }
    }

    fn from_main_cross(main_axis: Axis, main: f64, cross: f64) -> Self {
        match main_axis {
            Axis::Horizontal => Size {
                width: main,
                height: cross,
            },
            Axis::Vertical => Size {
                width: cross,
                height: main,
            },
        }
    }
}

trait AxisOffsetHelper {
    fn set_main_axis_offset(&mut self, main_axis: Axis, offset: f64);
    fn set_cross_axis_offset(&mut self, main_axis: Axis, offset: f64);
}

impl AxisOffsetHelper for Vec2 {
    fn set_main_axis_offset(&mut self, main_axis: Axis, offset: f64) {
        match main_axis {
            Axis::Horizontal => self.x = offset,
            Axis::Vertical => self.y = offset,
        }
    }

    fn set_cross_axis_offset(&mut self, main_axis: Axis, offset: f64) {
        match main_axis {
            Axis::Horizontal => self.y = offset,
            Axis::Vertical => self.x = offset,
        }
    }
}

fn main_cross_constraints(axis: Axis, min_main: f64, max_main: f64, min_cross: f64, max_cross: f64) -> BoxConstraints {
    match axis {
        Axis::Horizontal => BoxConstraints {
            min: Size {
                width: min_main,
                height: min_cross,
            },
            max: Size {
                width: max_main,
                height: max_cross,
            },
        },
        Axis::Vertical => BoxConstraints {
            min: Size {
                width: min_cross,
                height: min_main,
            },
            max: Size {
                width: max_cross,
                height: max_main,
            },
        },
    }
}

pub struct FlexLayoutParams {
    pub axis: Axis,
    /// When bounded, the flex item will take the maximum size, otherwise it will size to its content.
    pub constraints: BoxConstraints,
    pub cross_axis_alignment: CrossAxisAlignment,
    pub main_axis_alignment: MainAxisAlignment,
}

// Conforming to CSS:
// - flex layout gets either a size or `auto`

pub fn do_flex_layout(p: &FlexLayoutParams, children: &[AnyVisual]) -> Geometry {
    let axis = p.axis;
    let (main_axis_min, main_axis_max, mut cross_axis_min, cross_axis_max) = if axis == Axis::Horizontal {
        (
            p.constraints.min.width,
            p.constraints.max.width,
            p.constraints.min.height,
            p.constraints.max.height,
        )
    } else {
        (
            p.constraints.min.height,
            p.constraints.max.height,
            p.constraints.min.width,
            p.constraints.max.width,
        )
    };

    // stretch constraints
    //if p.cross_axis_alignment == CrossAxisAlignment::Stretch {
    //    cross_axis_min = cross_axis_max;
    //}

    let child_count = children.len();

    let mut flex_factors = Vec::with_capacity(child_count);
    for child in children.iter() {
        if let Some(flex) = FlexFactor.get(&*child.0) {
            flex_factors.push(flex);
        } else {
            flex_factors.push(0.0);
        }
    }

    let flex_sum: f64 = flex_factors.iter().sum(); // sum of flex factors

    let mut non_flex_main_total = 0.0; // total size of inflexible children
    let mut child_geoms = vec![Geometry::ZERO; child_count];
    let mut child_offsets = vec![Vec2::ZERO; child_count];

    // Layout each child with a zero flex factor (i.e. they don't expand along the main axis, they get their natural size instead)
    for (i, child) in children.iter().enumerate() {
        if flex_factors[i] == 0.0 {
            // layout child with unbounded main axis constraints and the incoming cross axis constraints
            let child_constraints = main_cross_constraints(axis, 0.0, f64::INFINITY, 0.0, cross_axis_max);
            child_geoms[i] = child.do_layout(&child_constraints);
            non_flex_main_total += child_geoms[i].size.main_length(axis);
        }
    }

    // Divide the remaining main axis space among the children with non-zero flex factors
    let remaining_main = main_axis_max - non_flex_main_total;
    for (i, child) in children.iter().enumerate() {
        if flex_factors[i] != 0.0 {
            let main_size = remaining_main * flex_factors[i] / flex_sum;
            // pass loose constraints along the main axis; it's the child's job to decide whether to fill the space or not
            let child_constraints = main_cross_constraints(axis, 0.0, main_size, 0.0, cross_axis_max);
            child_geoms[i] = child.do_layout(&child_constraints);
        }
    }

    // Determine the main-axis extent.
    let main_axis_content_size: f64 = child_geoms.iter().map(|g| g.size.main_length(axis)).sum();
    let main_axis_size = main_axis_content_size.max(main_axis_min).min(main_axis_max);
    let blank_space = main_axis_size - main_axis_content_size;

    // Position the children, depending on main axis alignment
    let space = match p.main_axis_alignment {
        MainAxisAlignment::SpaceBetween => blank_space / (child_count - 1) as f64,
        MainAxisAlignment::SpaceAround => blank_space / child_count as f64,
        MainAxisAlignment::SpaceEvenly => blank_space / (child_count + 1) as f64,
        MainAxisAlignment::Center | MainAxisAlignment::Start | MainAxisAlignment::End => 0.0,
    };
    let mut offset = match p.main_axis_alignment {
        MainAxisAlignment::SpaceBetween => 0.0,
        MainAxisAlignment::SpaceAround => space / 2.0,
        MainAxisAlignment::SpaceEvenly => space,
        MainAxisAlignment::Center => blank_space / 2.0,
        MainAxisAlignment::Start => 0.0,
        MainAxisAlignment::End => blank_space,
    };

    for (i, _) in children.iter().enumerate() {
        child_offsets[i].set_main_axis_offset(axis, offset);
        offset += child_geoms[i].size.main_length(axis) + space;
    }

    let cross_axis_content_size = child_geoms
        .iter()
        .map(|g| g.size.cross_length(axis))
        .reduce(f64::max)
        .unwrap();
    let cross_axis_size = cross_axis_content_size.clamp(cross_axis_min, cross_axis_max);

    /*let mut max_baseline: f64 = 0.0;
    for c in child_geoms.iter() {
        let cb = c.baseline.unwrap_or(c.size.cross_length(axis));
        max_baseline = max_baseline.max(cb);
    }

    let max_cross_axis_size_baseline_aligned = child_geoms
        .iter()
        .map(|g| {
            let size = g.size.cross_length(axis);
            size + (max_baseline - g.baseline.unwrap_or(size))
        })
        .reduce(f64::max)
        .unwrap();

    let cross_axis_size = match p.cross_axis_alignment {
        CrossAxisAlignment::Baseline => max_cross_axis_size_baseline_aligned,
        _ => max_cross_axis_size,
    };*/


    // Position the children on the cross axis
    for (i, c) in children.iter().enumerate() {
        let size = child_geoms[i].size.cross_length(axis);
        let offset = match p.cross_axis_alignment {
            CrossAxisAlignment::Start => 0.0,
            CrossAxisAlignment::End => cross_axis_size - size,
            CrossAxisAlignment::Center => (cross_axis_size - size) / 2.0,
            CrossAxisAlignment::Stretch => 0.0,
            CrossAxisAlignment::Baseline => {
                0.0 // TODO
                /*let baseline = child_geoms[i].baseline.unwrap_or(size);
                max_baseline - baseline*/
            }
        };
        child_offsets[i].set_cross_axis_offset(axis, offset);
        c.set_offset(child_offsets[i]);
    }

    let size = Size::from_main_cross(axis, main_axis_size, cross_axis_size);
    Geometry {
        size,
        baseline: Some(0.0),
        bounding_rect: Rect::from_origin_size(Point::ORIGIN, size),
        paint_bounding_rect: Rect::from_origin_size(Point::ORIGIN, size),
    }
}
