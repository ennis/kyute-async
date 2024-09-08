//! Skia utilities for its direct 3D backend
use std::cell::RefCell;
use std::mem;

use skia_safe as sk;
use skia_safe::{ColorSpace, ColorType, SurfaceProps};
use skia_safe::gpu::d3d::TextureResourceInfo;
use skia_safe::gpu::Protected;
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use windows::Win32::Graphics::Direct3D12::{D3D12_RESOURCE_STATE_RENDER_TARGET, ID3D12Resource};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;

use crate::backend::ApplicationBackend;
use crate::Size;

pub(crate) struct DrawingBackend {
    pub(crate) direct_context: Option<RefCell<sk::gpu::DirectContext>>,
}

impl DrawingBackend {


    /*/// Flushes commands on the specified surface.
    ///
    /// # Arguments
    ///
    /// * surface the surface returned by `create_surface_for_vulkan_image`
    /// * image handle to the vulkan image backing the surface (the one that was passed to `create_surface_for_vulkan_image`)
    ///
    /// # Safety
    ///
    /// * `image` must specify a valid image (must not have been deleted)
    /// * `surface` must have been created by `create_surface_for_vulkan_image`
    /// * `image` must be the backing image for `surface` as specified in a prior call to `create_surface_for_vulkan_image`.
    pub(crate) unsafe fn flush_surface_for_vulkan_image(&mut self, mut surface: sk::Surface, image: graal::ImageInfo) {
        // flush the GPU frame
        //let _span = trace_span!("Flush skia surface").entered();
        let mut frame = graal::Frame::new();
        let pass = graal::PassBuilder::new()
            .name("flush SkSurface")
            .image_dependency(
                // FIXME we just assume how it's going to be used by skia
                image.id,
                graal::vk::AccessFlags::MEMORY_READ | graal::vk::AccessFlags::MEMORY_WRITE,
                graal::vk::PipelineStageFlags::ALL_COMMANDS,
                graal::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                graal::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            )
            .submit_callback(move |_cctx, _, _queue| {
                surface.flush_and_submit();
            });
        frame.add_pass(pass);
        self.context.submit_frame(&mut (), frame, &Default::default());
    }*/
}
