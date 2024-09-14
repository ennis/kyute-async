//! Frame containers
use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;

use kurbo::{Insets, RoundedRect, Size};
use palette::cam16::Cam16IntoUnclamped;

use crate::{Color, drawing, PaintCtx, skia};
use crate::drawing::{BoxShadow, Paint, ToSkia};
use crate::element::{AnyVisual, Element, Visual};
use crate::event::Event;
use crate::handler::Handler;
use crate::layout::{Alignment, BoxConstraints, Geometry, LengthOrPercentage, place_child_box, flex::do_flex_layout};
use crate::layout::flex::{Axis, CrossAxisAlignment, FlexLayoutParams, MainAxisAlignment};
use crate::style::{Active, BackgroundColor, Baseline, BorderBottom, BorderColor, BorderLeft, BorderRadius, BorderRight, BorderTop, BoxShadows, Focus, Height, HorizontalAlign, Hover, PaddingBottom, PaddingLeft, PaddingRight, PaddingTop, Style, VerticalAlign, Width};

#[derive(Clone, Default)]
pub struct ResolvedFrameStyle {
    padding_left: LengthOrPercentage,
    padding_right: LengthOrPercentage,
    padding_top: LengthOrPercentage,
    padding_bottom: LengthOrPercentage,
    horizontal_align: Alignment,
    vertical_align: Alignment,
    baseline: Option<LengthOrPercentage>,
    width: Option<LengthOrPercentage>,
    height: Option<LengthOrPercentage>,
    border_left: LengthOrPercentage,
    border_right: LengthOrPercentage,
    border_top: LengthOrPercentage,
    border_bottom: LengthOrPercentage,
    border_color: Color,
    border_radius: f64,
    background_color: Color,
    shadows: Vec<BoxShadow>,
    direction: Axis,
    main_axis_alignment: MainAxisAlignment,
    cross_axis_alignment: CrossAxisAlignment,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct InteractState {
    pub active: bool,
    pub hovered: bool,
    pub focused: bool,
}

/// A container with a fixed width and height, into which a unique widget is placed.
pub struct Frame {
    element: Element,
    pub clicked: Handler<()>,
    pub hovered: Handler<bool>,
    pub active: Handler<bool>,
    pub focused: Handler<bool>,
    pub state_changed: Handler<InteractState>,
    state: Cell<InteractState>,
    style: Style,
    style_changed: Cell<bool>,
    state_affects_style: Cell<bool>,
    resolved_style: RefCell<ResolvedFrameStyle>,
}

impl Deref for Frame {
    type Target = Element;

    fn deref(&self) -> &Self::Target {
        &self.element
    }
}

impl Frame {
    /// Creates a new `Frame` with the given decoration.
    pub fn new(style: Style) -> Rc<Frame> {
        Element::new_derived(|element| Frame {
            element,
            clicked: Default::default(),
            hovered: Default::default(),
            active: Default::default(),
            focused: Default::default(),
            state_changed: Default::default(),
            state: Cell::new(Default::default()),
            style,
            style_changed: Cell::new(true),
            state_affects_style: Cell::new(false),
            resolved_style: Default::default(),
        })
    }

    pub fn set_content(&self, content: &dyn Visual) {
        (self as &dyn Visual).add_child(content);
    }

    fn calculate_style(&self) {
        if self.style_changed.get() {
            let state = self.state.get();
            let mut s = self.style.clone();
            let mut state_affects_style = false;

            if let Some(focused) = self.style.get(Focus) {
                if state.focused {
                    s = focused.over(s);
                }
                state_affects_style = true;
            }
            if let Some(hovered) = self.style.get(Hover) {
                if state.hovered {
                    s = hovered.over(s);
                }
                state_affects_style = true;
            }
            if let Some(active) = self.style.get(Active) {
                if state.active {
                    s = active.over(s);
                }
                state_affects_style = true;
            }

            let mut r = self.resolved_style.borrow_mut();
            r.padding_left = s.get_or_default(PaddingLeft);
            r.padding_right = s.get_or_default(PaddingRight);
            r.padding_top = s.get_or_default(PaddingTop);
            r.padding_bottom = s.get_or_default(PaddingBottom);
            r.horizontal_align = s.get_or_default(HorizontalAlign);
            r.vertical_align = s.get_or_default(VerticalAlign);
            r.baseline = s.get(Baseline);
            r.width = s.get(Width);
            r.height = s.get(Height);
            r.border_left = s.get_or_default(BorderLeft);
            r.border_right = s.get_or_default(BorderRight);
            r.border_top = s.get_or_default(BorderTop);
            r.border_bottom = s.get_or_default(BorderBottom);
            r.border_color = s.get_or_default(BorderColor);
            r.border_radius = s.get_or_default(BorderRadius);
            r.shadows = s.get_or_default(BoxShadows);
            r.direction = Axis::Horizontal;
            r.main_axis_alignment = MainAxisAlignment::Start;
            r.cross_axis_alignment = CrossAxisAlignment::Start;
            r.background_color = s.get_or_default(BackgroundColor);

            self.state_affects_style.set(state_affects_style);
            self.style_changed.set(false);
        }
    }
}

struct FrameSizes {
    parent_min: f64,
    parent_max: f64,
    fixed: Option<f64>,
    padding_before: f64,
    padding_after: f64,
}

impl FrameSizes {
    fn compute_child_constraint(&self) -> (f64, f64) {
        let mut min = self.parent_min;
        let mut max = self.parent_max;
        if let Some(fixed) = self.fixed {
            max = fixed;
        }
        max -= self.padding_before + self.padding_after;
        max = max.min(self.parent_max);
        min = min.min(max);
        (min, max)
    }

    fn compute_self_size(&self, child_len: f64) -> f64 {
        let mut size = child_len;
        if let Some(fixed) = self.fixed {
            size = fixed;
        } else {
            size += self.padding_before + self.padding_after;
        }
        // this may be redundant
        size = size.clamp(self.parent_min, self.parent_max);
        size
    }
}

impl Visual for Frame {
    fn element(&self) -> &Element {
        &self.element
    }

    fn layout(&self, children: &[AnyVisual], constraints: &BoxConstraints) -> Geometry {

        self.calculate_style();
        let s = self.resolved_style.borrow();

        let max_width = constraints.max.width;
        let max_height = constraints.max.height;

        let horizontal = FrameSizes {
            parent_min: constraints.min.width,
            parent_max: constraints.max.width,
            fixed: s.width.map(|w| w.resolve(max_width)),
            padding_before: s.padding_left.resolve(max_width),
            padding_after: s.padding_right.resolve(max_width),
        };

        let vertical = FrameSizes {
            parent_min: constraints.min.height,
            parent_max: constraints.max.height,
            fixed: s.height.map(|h| h.resolve(max_height)),
            padding_before: s.padding_top.resolve(max_height),
            padding_after: s.padding_bottom.resolve(max_height),
        };

        let (child_min_width, child_max_width) = horizontal.compute_child_constraint();
        let (child_min_height, child_max_height) = vertical.compute_child_constraint();

        let child_constraints = BoxConstraints {
            min: Size::new(child_min_width, child_min_height),
            max: Size::new(child_max_width, child_max_height),
        };

        // layout children
        // TODO other layouts
        let flex_params = FlexLayoutParams {
            axis: s.direction,
            constraints: child_constraints,
            cross_axis_alignment: s.cross_axis_alignment,
            main_axis_alignment: s.main_axis_alignment,
        };
        let child_geom = do_flex_layout(&flex_params, children);

        // child geometry is determined, now determine our size
        let self_width = horizontal.compute_self_size(child_geom.size.width);
        let self_height = vertical.compute_self_size(child_geom.size.height);

        // position the content within the frame
        let baseline = s.baseline.map(|b| b.resolve(self_height));
        let offset = place_child_box(
            child_geom.size,
            child_geom.baseline,
            Size::new(self_width, self_height),
            baseline,
            s.horizontal_align,
            s.vertical_align,
            &Insets::new(
                horizontal.padding_before,
                vertical.padding_before,
                horizontal.padding_after,
                vertical.padding_after,
            ),
        );
        for child in children.iter() {
            let mut t = child.transform();
            // TODO not sure about the order here
            t = t.then_translate(offset);
            child.set_transform(t);
        }

        // our baseline
        let baseline = baseline
            .or(child_geom.baseline.map(|b| b + offset.y))
            .unwrap_or(self_height);
        let size = Size::new(self_width, self_height);
        Geometry {
            size,
            baseline: Some(baseline),
            bounding_rect: size.to_rect(),       // TODO
            paint_bounding_rect: size.to_rect(), // TODO
        }
    }

    async fn event(&self, event: &mut Event)
    where
        Self: Sized,
    {
        async fn update_state(this: &Frame, state: InteractState) {
            this.state.set(state);
            this.state_changed.emit(state).await;
            if this.state_affects_style.get() {
                this.style_changed.set(true);
                this.mark_needs_relayout();
            }
        }

        let mut state = self.state.get();
        match event {
            Event::PointerDown(_) => {
                state.active = true;
                update_state(self, state).await;
                self.active.emit(true).await;
            }
            Event::PointerUp(_) => {
                if state.active {
                    state.active = false;
                    update_state(self, state).await;
                    self.clicked.emit(()).await;
                }
            }
            Event::PointerEnter(_) => {
                state.hovered = true;
                update_state(self, state).await;
                self.hovered.emit(true).await;
            }
            Event::PointerLeave(_) => {
                state.hovered = false;
                update_state(self, state).await;
                self.hovered.emit(false).await;
            }
            _ => {}
        }
    }

    fn paint(&self, ctx: &mut PaintCtx) {
        let size = self.element.geometry().size;
        let rect = size.to_rect();
        let s = self.resolved_style.borrow();
        let insets = Insets::new(
            s.border_left.resolve(size.width),
            s.border_top.resolve(size.height),
            s.border_right.resolve(size.width),
            s.border_bottom.resolve(size.height),
        );
        // border shape
        let inner_shape = RoundedRect::from_rect(rect - insets, s.border_radius - 0.5 * insets.x_value());
        let outer_shape = RoundedRect::from_rect(rect, s.border_radius);

        ctx.with_canvas(|canvas| {
            // draw drop shadows
            for shadow in &s.shadows {
                if !shadow.inset {
                    drawing::draw_box_shadow(canvas, &outer_shape, shadow);
                }
            }

            // fill
            let mut paint = Paint::Color(s.background_color).to_sk_paint(rect);
            paint.set_style(skia::paint::Style::Fill);
            canvas.draw_rrect(inner_shape.to_skia(), &paint);

            // draw inset shadows
            for shadow in &s.shadows {
                if shadow.inset {
                    drawing::draw_box_shadow(canvas, &inner_shape, shadow);
                }
            }

            // paint border
            if s.border_color.alpha() != 0.0 {
                let mut paint = Paint::Color(s.border_color).to_sk_paint(rect);
                paint.set_style(skia::paint::Style::Fill);
                canvas.draw_drrect(outer_shape.to_skia(), inner_shape.to_skia(), &paint);
            }
        });
    }
}

#[test]
fn test_im() {
    let mut ordmap_1 = imbl::ordmap![
        1 => "a",
        2 => "b",
        3 => "c"
    ];
    let ordmap_2 = imbl::ordmap![
        1 => "d"
        //2 => "e"
        //3 => "f"
    ];

    //let mut ordmap_1 = im::ordmap!{1 => 1, 3 => 3};
    //let ordmap_2 = im::ordmap!{2 => 2, 3 => 4};


    ordmap_1 = ordmap_2.union(ordmap_1);

    dbg!(ordmap_1);

}