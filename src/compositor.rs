//! System compositor interface
use raw_window_handle::RawWindowHandle;
use skia_safe as sk;

use crate::{backend, Size};
use crate::app_globals::AppGlobals;

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
    ///
    /// # Panics
    /// - if the DrawableSurface returned by the last call to `acquire_drawing_surface` has not been dropped (TODO)
    ///
    pub fn acquire_drawing_surface(&self) -> DrawableSurface {
        // In theory, we could return a DrawableSurface that mutably borrows the Layer, to
        // statically prevent calling `acquire_drawing_surface` when a DrawableSurface is alive.
        // However, this would lock all `&self` methods for the duration of the borrow, which
        // is not very ergonomic (methods like `size()` would be inaccessible, even though
        // it's perfectly OK to call while a DrawableSurface is active).
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

    /// Creates a drawable surface layer.
    ///
    /// Use `acquire_drawing_surface` to obtain a drawable surface with a Skia context from the layer.
    ///
    /// # Arguments
    ///
    /// * size Size of the surface in pixels
    /// * format Pixel format
    pub fn new_surface(size: Size, format: ColorType) -> Layer {
        Layer(AppGlobals::get().backend.create_surface_layer(size, format))
    }
}


