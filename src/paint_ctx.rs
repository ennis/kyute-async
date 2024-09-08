use crate::compositor::DrawableSurface;
use crate::drawing::ToSkia;
use kurbo::{Affine, Rect, Vec2};

/// Paint context.
pub struct PaintCtx<'a> {
    pub(crate) scale_factor: f64,
    /// Transform from window area to the current element.
    pub(crate) window_transform: Affine,
    /// Drawable surface.
    pub surface: &'a DrawableSurface,
    //pub(crate) debug_info: PaintDebugInfo,
}

impl<'a> PaintCtx<'a> {
    pub fn with_offset<F, R>(&mut self, offset: Vec2, f: F) -> R
    where
        F: FnOnce(&mut PaintCtx<'a>) -> R,
    {
        self.with_transform(&Affine::translate(offset), f)
    }

    pub fn with_transform<F, R>(&mut self, transform: &Affine, f: F) -> R
    where
        F: FnOnce(&mut PaintCtx<'a>) -> R,
    {
        let scale = self.scale_factor as skia_safe::scalar;
        let prev_transform = self.window_transform;
        self.window_transform *= *transform;
        let mut surface = self.surface.surface();
        surface.canvas().save();
        surface.canvas().reset_matrix();
        surface.canvas().scale((scale, scale));
        surface.canvas().concat(&self.window_transform.to_skia());
        // TODO clip
        let result = f(self);
        let mut surface = self.surface.surface();
        surface.canvas().restore();
        self.window_transform = prev_transform;

        result
    }

    pub fn with_clip_rect(&mut self, rect: Rect, f: impl FnOnce(&mut PaintCtx<'a>)) {
        let mut surface = self.surface.surface();
        surface.canvas().save();
        surface
            .canvas()
            .clip_rect(rect.to_skia(), skia_safe::ClipOp::Intersect, false);
        f(self);
        surface.canvas().restore();
    }

    /*pub fn paint(&mut self, widget: &mut dyn Widget) {
        /*#[cfg(debug_assertions)]
        {
            self.debug_info.add(PaintElementDebugInfo {
                element_ptr: elem_ptr_id(child_element),
                transform: self.window_transform,
            });
        }*/
        widget.paint(self)
    }*/

    pub fn with_canvas<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&skia_safe::Canvas) -> R,
    {
        let mut surface = self.surface.surface();
        let result = f(surface.canvas());
        result
    }
}
