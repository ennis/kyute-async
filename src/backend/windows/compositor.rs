//! Windows compositor implementation details

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ops::Deref;
use std::rc::Rc;

use raw_window_handle::RawWindowHandle;
use skia_safe as sk;
use skia_safe::gpu::d3d::TextureResourceInfo;
use skia_safe::gpu::{DirectContext, FlushInfo, Protected};
use skia_safe::surface::BackendSurfaceAccess;
use skia_safe::{ColorSpace, SurfaceProps};
use slotmap::SecondaryMap;
use tracy_client::span;
use windows::core::{Interface, Owned, BSTR};
use windows::Foundation::Numerics::Vector2;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12Object, ID3D12Resource, D3D12_FENCE_FLAG_NONE,
    D3D12_RESOURCE_STATE_RENDER_TARGET,
};
use windows::Win32::Graphics::DirectComposition::{IDCompositionDesktopDevice, IDCompositionDevice3, IDCompositionTarget, IDCompositionVisual3};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_MODE_SCALING_UNSPECIFIED, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{DXGIGetDebugInterface1, IDXGIDebug1, IDXGIFactory3, IDXGISwapChain3, DXGI_DEBUG_ALL, DXGI_DEBUG_RLO_DETAIL, DXGI_PRESENT, DXGI_SCALING_ASPECT_RATIO_STRETCH, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionStretch, Compositor as WinCompositor, ContainerVisual, Visual};

use crate::app_globals::AppGlobals;
use crate::backend::windows::BackendInner;
use crate::backend::ApplicationBackend;
use crate::compositor::ColorType;
use crate::skia_backend::DrawingBackend;
use crate::{backend, Size};

////////////////////////////////////////////////////////////////////////////////////////////////////

const SWAP_CHAIN_BUFFER_COUNT: u32 = 2;

struct CompositorData {
    compositor: WinCompositor,
    direct_context: RefCell<sk::gpu::DirectContext>,
}

/// Windows drawable surface backend.
pub(crate) struct DrawableSurface {
    composition_device: IDCompositionDesktopDevice,
    context: DirectContext,
    swap_chain: IDXGISwapChain3,
    surface: sk::Surface,
}

impl DrawableSurface {
    pub(crate) fn surface(&self) -> sk::Surface {
        self.surface.clone()
    }

    fn present(&mut self) {
        {
            let _span = span!("skia: flush_and_submit");
            self.context.flush_surface_with_access(
                &mut self.surface,
                BackendSurfaceAccess::Present,
                &FlushInfo::default(),
            );
            self.context.submit(None);
        }

        unsafe {
            let _span = span!("D3D12: present");
            self.swap_chain.Present(1, DXGI_PRESENT::default()).unwrap();
            self.composition_device.Commit().unwrap();
        }


        if let Some(client) = tracy_client::Client::running() {
            client.frame_mark();
        }
    }
}

impl Drop for DrawableSurface {
    fn drop(&mut self) {
        self.present();
    }
}

/// Swap chain abstraction that also manages a wait object for frame latency.
struct SwapChain {
    inner: IDXGISwapChain3,
    frame_latency_waitable: Owned<HANDLE>,
}

/// Compositor layer.
pub struct Layer {
    app: Rc<BackendInner>,
    visual: IDCompositionVisual3,
    size: Cell<Size>,
    swap_chain: Option<SwapChain>,
    window_target: RefCell<Option<IDCompositionTarget>>,
}

impl Drop for Layer {
    fn drop(&mut self) {
        self.app.wait_for_gpu();
    }
}

impl Layer {
    /// Resizes a surface layer.
    pub(crate) fn set_surface_size(&self, size: Size) {
        // skip if same size
        if self.size.get() == size {
            return;
        }

        let width = size.width as u32;
        let height = size.height as u32;
        // avoid resizing to zero width
        if width == 0 || height == 0 {
            return;
        }

        self.size.set(size);

        if let Some(ref swap_chain) = self.swap_chain {
            // Wait for the GPU to finish using the previous swap chain buffers.
            self.app.wait_for_gpu();
            // Skia may still hold references to swap chain buffers which would prevent
            // ResizeBuffers from succeeding. This cleans them up.
            self.app.direct_context.borrow_mut().flush_submit_and_sync_cpu();

            unsafe {
                // SAFETY: basic FFI call
                match swap_chain.inner.ResizeBuffers(
                    SWAP_CHAIN_BUFFER_COUNT,
                    width,
                    height,
                    DXGI_FORMAT_R16G16B16A16_FLOAT,
                    DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
                ) {
                    Ok(_) => {}
                    Err(hr) => {
                        //let removed_reason = self.device.GetDeviceRemovedReason().unwrap_err();
                        panic!("IDXGISwapChain::ResizeBuffers failed: {}", hr);
                    }
                }
            }
        }
    }

    /// Waits for the specified surface to be ready for presentation.
    ///
    /// TODO explain
    pub(crate) fn wait_for_presentation(&self) {
        let _span = span!("wait_for_surface");
        let swap_chain = self.swap_chain.as_ref().expect("layer should be a surface layer");
        // if the swapchain has a mechanism for syncing with presentation, use it,
        // otherwise do nothing.
        if !swap_chain.frame_latency_waitable.is_invalid() {
            unsafe {
                WaitForSingleObject(*swap_chain.frame_latency_waitable, 1000);
            }
        }
    }

    /// Creates a skia drawing context for the specified surface layer.
    pub(crate) fn acquire_drawing_surface(&self) -> DrawableSurface {
        let swap_chain = self.swap_chain.as_ref().expect("layer should be a surface layer");

        unsafe {
            // acquire next image from swap chain
            let index = swap_chain.inner.GetCurrentBackBufferIndex();
            let swap_chain_buffer = swap_chain
                .inner
                .GetBuffer::<ID3D12Resource>(index)
                .expect("failed to retrieve swap chain buffer");

            let surface = self.app.create_surface_for_texture(
                swap_chain_buffer,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                self.size.get(),
                sk::gpu::SurfaceOrigin::TopLeft,
                sk::ColorType::RGBAF16,
                sk::ColorSpace::new_srgb_linear(),
                Some(sk::SurfaceProps::new(
                    sk::SurfacePropsFlags::default(),
                    sk::PixelGeometry::RGBH,
                )),
            );
            DrawableSurface {
                composition_device: self.app.composition_device.clone(),
                context: self.app.direct_context.borrow().clone(),
                surface,
                swap_chain: swap_chain.inner.clone(),
            }
        }
    }

    /// Binds a composition layer to a window.
    ///
    /// # Safety
    ///
    /// The window handle is valid.
    ///
    /// TODO: return result
    pub(crate) unsafe fn bind_to_window(&self, window: RawWindowHandle) {
        let win32_handle = match window {
            RawWindowHandle::Win32(w) => w,
            _ => panic!("expected a Win32 window handle"),
        };
        let window_target = self
            .app
            .composition_device
            .CreateTargetForHwnd(HWND(win32_handle.hwnd.get() as *mut c_void), false)
            .expect("CreateTargetForHwnd failed");
        window_target.SetRoot(&self.visual).expect("SetRoot failed");
        self.window_target.replace(Some(window_target));
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Compositor impl
////////////////////////////////////////////////////////////////////////////////////////////////////

impl BackendInner {
    /// Creates a surface backed by the specified D3D texture resource.
    ///
    /// # Safety
    ///
    /// The parameters must match the properties of the vulkan image:
    ///
    /// * `format`, `size` must be the same as specified during creation of the image
    /// * `color_type` must be compatible with `format`
    ///
    /// TODO: other preconditions
    unsafe fn create_surface_for_texture(
        &self,
        image: ID3D12Resource,
        format: DXGI_FORMAT,
        size: Size,
        surface_origin: sk::gpu::SurfaceOrigin,
        color_type: skia_safe::ColorType,
        color_space: ColorSpace,
        surface_props: Option<SurfaceProps>,
    ) -> sk::Surface {
        let texture_resource_info = TextureResourceInfo {
            resource: image,
            alloc: None,
            resource_state: D3D12_RESOURCE_STATE_RENDER_TARGET, // FIXME: either pass in parameters or document assumption
            format,
            sample_count: 1, // FIXME pass in parameters
            level_count: 1,  // FIXME pass in parameters
            sample_quality_pattern: 0,
            protected: Protected::No,
        };

        let backend_render_target =
            sk::gpu::BackendRenderTarget::new_d3d((size.width as i32, size.height as i32), &texture_resource_info);
        let direct_context = &mut *self.direct_context.borrow_mut();
        let sk_surface = skia_safe::gpu::surfaces::wrap_backend_render_target(
            direct_context,
            &backend_render_target,
            surface_origin,
            color_type,
            color_space,
            surface_props.as_ref(),
        )
        .expect("skia surface creation failed");
        sk_surface
    }
}

impl ApplicationBackend {
    /// Creates a surface layer.
    ///
    /// FIXME: don't ignore format
    pub(crate) fn create_surface_layer(&self, size: Size, _format: ColorType) -> Layer {
        unsafe {
            // Create the swap chain backing the layer
            let width = size.width as u32;
            let height = size.height as u32;

            assert!(width != 0 && height != 0, "surface layer cannot be zero-sized");

            // create swap chain
            let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: SWAP_CHAIN_BUFFER_COUNT,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            };

            // SAFETY: FFI
            let swap_chain: IDXGISwapChain3 = self
                .0
                .dxgi_factory
                .CreateSwapChainForComposition(&*self.0.command_queue, &swap_chain_desc, None)
                .expect("CreateSwapChainForComposition failed")
                .cast::<IDXGISwapChain3>()
                .unwrap();

            // SAFETY: FFI
            swap_chain.SetMaximumFrameLatency(1).unwrap();
            let frame_latency_waitable = swap_chain.GetFrameLatencyWaitableObject();

            let swap_chain = SwapChain {
                inner: swap_chain,
                // SAFETY: we own the handle
                frame_latency_waitable: { Owned::new(frame_latency_waitable) },
            };

            // Create the composition surface representing the swap chain in the compositor


            // Create the visual+brush holding the surface
            let visual = self.0.composition_device.CreateVisual().unwrap();
            visual.SetContent(&swap_chain.inner).unwrap();

            Layer {
                app: self.0.clone(),
                visual: visual.cast().unwrap(),
                size: Cell::new(size),
                swap_chain: Some(swap_chain),
                window_target: RefCell::new(None),
            }
        }
    }
}
