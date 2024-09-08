//! System compositor interface
//!
//! TODO: Rc handles for layers (Rc<Compositor>)
//! TODO: DrawableSurface should have Rc handle semantics
use crate::{backend, Size};
use raw_window_handle::RawWindowHandle;
use skia_safe as sk;
use slotmap::{SecondaryMap, SlotMap};
use std::cell::RefCell;

////////////////////////////////////////////////////////////////////////////////////////////////////

slotmap::new_key_type! {
    /// Unique identifier for a compositor layer.
    pub struct LayerID;
}

////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Copy, Clone, Debug)]
struct LayerInfo {}

#[derive(Copy, Clone, Debug)]
struct TreeInfo {
    parent: Option<LayerID>,
    prev_sibling: Option<LayerID>,
    next_sibling: Option<LayerID>,
}

#[derive(Copy, Clone, Debug, Default)]
struct ContainerInfo {
    first_child: Option<LayerID>,
    last_child: Option<LayerID>,
}

#[derive(Copy, Clone, Debug)]
struct EffectInfo {
    opacity: f32,
}

#[derive(Copy, Clone, Debug)]
struct TransformInfo {
    transform: kurbo::Affine,
}

#[derive(Copy, Clone, Debug)]
struct ClipLayer {
    bounds: kurbo::Rect,
}

#[derive(Copy, Clone, Debug)]
struct SurfaceInfo {}

////////////////////////////////////////////////////////////////////////////////////////////////////

/// A drawable surface
pub struct DrawableSurface {
    backend: backend::DrawableSurface,
}

impl DrawableSurface {
    /// Returns the underlying skia surface.
    pub fn surface(&self) -> sk::Surface {
        self.backend.surface()
    }
}

/// Pixel format of a drawable surface.
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
pub enum ColorType {
    Alpha8,
    RGBA8888,
    BGRA8888,
    RGBA1010102,
    BGRA1010102,
    RGB101010x,
    BGR101010x,
    BGR101010xXR,
    Gray8,
    RGBAF16,
    RGBAF32,
    A16Float,
    A16UNorm,
    R16G16B16A16UNorm,
    SRGBA8888,
    R8UNorm,
}

impl ColorType {
    pub fn to_skia_color_type(&self) -> sk::ColorType {
        match *self {
            //ColorType::Unknown => sk::ColorType::Unknown,
            ColorType::Alpha8 => sk::ColorType::Alpha8,
            //ColorType::RGB565 => sk::ColorType::RGB565,
            //ColorType::ARGB4444 => sk::ColorType::ARGB4444,
            ColorType::RGBA8888 => sk::ColorType::RGBA8888,
            //ColorType::RGB888x => sk::ColorType::RGB888x,
            ColorType::BGRA8888 => sk::ColorType::BGRA8888,
            ColorType::RGBA1010102 => sk::ColorType::RGBA1010102,
            ColorType::BGRA1010102 => sk::ColorType::BGRA1010102,
            ColorType::RGB101010x => sk::ColorType::RGB101010x,
            ColorType::BGR101010x => sk::ColorType::BGR101010x,
            ColorType::BGR101010xXR => sk::ColorType::BGR101010xXR,
            ColorType::Gray8 => sk::ColorType::Gray8,
            //ColorType::RGBAF16Norm => sk::ColorType::RGBAF16Norm,
            ColorType::RGBAF16 => sk::ColorType::RGBAF16,
            ColorType::RGBAF32 => sk::ColorType::RGBAF32,
            //ColorType::R8G8UNorm => sk::ColorType::R8G8UNorm,
            ColorType::A16Float => sk::ColorType::A16Float,
            //ColorType::R16G16Float => sk::ColorType::R16G16Float,
            ColorType::A16UNorm => sk::ColorType::A16UNorm,
            //ColorType::R16G16UNorm => sk::ColorType::R16G16UNorm,
            ColorType::R16G16B16A16UNorm => sk::ColorType::R16G16B16A16UNorm,
            ColorType::SRGBA8888 => sk::ColorType::SRGBA8888,
            ColorType::R8UNorm => sk::ColorType::R8UNorm,
        }
    }
}

/// Handle to a compositor layer.
#[derive(Clone)]
pub struct Layer(backend::Layer);

impl Layer {
    /// Waits for the specified surface to be ready for presentation.
    ///
    /// TODO explain
    pub fn wait_for_presentation(&self) {
        self.0.wait_for_presentation();
    }

    /// Creates a skia drawing context to paint on the specified surface layer.
    ///
    /// Only one drawing context can be active at a time.
    pub fn acquire_drawing_surface(&self) -> DrawableSurface {
        DrawableSurface {
            backend: self.0.acquire_drawing_surface(),
        }
    }

    /// Resizes a surface layer.
    pub fn set_surface_size(&self, size: Size) {
        self.0.set_surface_size(size);
    }

    /// Binds a layer to a native window.
    pub unsafe fn bind_to_window(&self, window: RawWindowHandle) {
        self.0.bind_to_window(window)
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

/// A connection to the system compositor.
pub struct Compositor { pub(crate) backend:  backend::Compositor }

impl Compositor {
    pub(crate) fn new(app_backend: &backend::AppBackend) -> Compositor {
        let backend = backend::Compositor::new(app_backend);
        Compositor {backend}
    }

    /// Creates a drawable surface layer.
    ///
    /// Use `acquire_drawing_surface` to obtain a drawable surface with a Skia context from the layer.
    ///
    /// # Arguments
    ///
    /// * size Size of the surface in pixels
    /// * format Pixel format
    pub fn create_surface_layer(&self, size: Size, format: ColorType) -> Layer {
        let b = self.backend.create_surface_layer(size, format);
        Layer(b)
    }
}
