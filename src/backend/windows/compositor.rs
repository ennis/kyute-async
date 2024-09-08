//! Windows compositor implementation details

use raw_window_handle::RawWindowHandle;
use skia_safe as sk;
use skia_safe::gpu::d3d::TextureResourceInfo;
use skia_safe::gpu::surfaces::wrap_backend_render_target;
use skia_safe::gpu::{FlushInfo, Protected};
use skia_safe::surface::BackendSurfaceAccess;
use skia_safe::{ColorSpace, SurfaceProps};
use slotmap::SecondaryMap;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;
use windows::Foundation::Numerics::Vector2;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12CommandQueue, ID3D12Device, ID3D12Fence, ID3D12Object, ID3D12Resource, D3D12_FENCE_FLAG_NONE,
    D3D12_RESOURCE_STATE_RENDER_TARGET,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIDebug1, IDXGIFactory3, IDXGISwapChain3, DXGI_DEBUG_ALL, DXGI_DEBUG_RLO_DETAIL,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::WinRT::Composition::{ICompositorDesktopInterop, ICompositorInterop};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{Compositor as WinCompositor, ContainerVisual, Visual};

use crate::app_globals::AppGlobals;
use crate::backend::windows::event::Win32Event;
use crate::backend::AppBackend;
use crate::compositor::{ColorType, LayerID};
use crate::skia_backend::DrawingBackend;
use crate::{backend, Size};
use tracy_client::span;
use windows::core::{Interface, Owned, BSTR};
use windows::Win32::Graphics::Dxgi::DXGI_PRESENT;

//mod swap_chain;

////////////////////////////////////////////////////////////////////////////////////////////////////

const SWAP_CHAIN_BUFFER_COUNT: u32 = 2;

/// Windows drawable surface backend
pub(crate) struct DrawableSurface {
    direct_context: RefCell<sk::gpu::DirectContext>,
    layer: Layer,
    surface: sk::Surface,
}

impl DrawableSurface {
    pub(crate) fn surface(&self) -> sk::Surface {
        self.surface.clone()
    }
}

impl Drop for DrawableSurface {
    fn drop(&mut self) {
        AppGlobals::get()
            .compositor
            .backend
            .flush_and_present(&self.layer.0.swap_chain.as_ref().unwrap().inner);
        //eprintln!("Drawable drop {}x{}", self.surface.width(), self.surface.height());
    }
}

/// Swap chain abstraction that also manages a wait object for frame latency.
struct SwapChain {
    inner: IDXGISwapChain3,
    frame_latency_waitable: Owned<HANDLE>,
}

/*
impl Drop for SwapChain {
    fn drop(&mut self) {
        if !self.frame_latency_waitable.is_invalid() {
            unsafe {
                CloseHandle(self.frame_latency_waitable);
            }
        }
    }
}*/

/// A windows compositor native layer (a `Visual`).
struct LayerInner {
    visual: Visual,
    size: Cell<Size>,
    swap_chain: Option<SwapChain>,
    window_target: RefCell<Option<DesktopWindowTarget>>,
}

impl Drop for LayerInner {
    fn drop(&mut self) {
        AppGlobals::get()
            .compositor
            .backend.wait_for_gpu()
    }
}

/// Compositor layer.
#[derive(Clone)]
pub struct Layer(Rc<LayerInner>);

impl Layer {
    /// Resizes a surface layer.
    pub(crate) fn set_surface_size(&self, size: Size) {
        let this = &self.0;

        let compositor = &AppGlobals::get().compositor.backend;

        // skip if same size
        if this.size.get() == size {
            return;
        }

        let width = size.width as u32;
        let height = size.height as u32;
        // avoid resizing to zero width
        if width == 0 || height == 0 {
            return;
        }

        /*unsafe {
            compositor.debug
                .ReportLiveObjects(DXGI_DEBUG_ALL, DXGI_DEBUG_RLO_DETAIL)
                .unwrap();
        }*/

        if let Some(ref swap_chain) = self.0.swap_chain {
            // Wait for the GPU to finish using the previous swap chain buffers.
            compositor.wait_for_gpu();
            // Skia may still hold references to swap chain buffers which would prevent
            // ResizeBuffers from succeeding. This cleans them up.
            compositor.direct_context.borrow_mut().flush_submit_and_sync_cpu();

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

        self.0.size.set(size);
        self.0
            .visual
            .SetSize(Vector2::new(size.width as f32, size.height as f32))
            .unwrap();
    }

    /// Waits for the specified surface to be ready for presentation.
    ///
    /// TODO explain
    pub(crate) fn wait_for_presentation(&self) {
        let _span = span!("wait_for_surface");
        let swap_chain = self.0.swap_chain.as_ref().expect("layer should be a surface layer");
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

        let swap_chain = self.0.swap_chain.as_ref().expect("layer should be a surface layer");
        let compositor = &AppGlobals::get().compositor.backend;
        let direct_context = compositor.direct_context.borrow().clone();

        unsafe {
            // acquire next image from swap chain
            let index = swap_chain.inner.GetCurrentBackBufferIndex();
            let swap_chain_buffer = swap_chain
                .inner
                .GetBuffer::<ID3D12Resource>(index)
                .expect("failed to retrieve swap chain buffer");

            /*swap_chain_buffer
                .cast::<ID3D12Object>()
                .unwrap()
                .SetName(&BSTR::from(format!(
                    "swap_chain_buffer {}x{}",
                    layer.size.width, layer.size.height
                )))
                .unwrap();*/

            let surface = compositor.create_surface_for_texture(
                swap_chain_buffer,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
                self.0.size.get(),
                sk::gpu::SurfaceOrigin::TopLeft,
                sk::ColorType::RGBAF16,
                sk::ColorSpace::new_srgb_linear(),
                Some(sk::SurfaceProps::new(
                    sk::SurfacePropsFlags::default(),
                    sk::PixelGeometry::RGBH,
                )),
            );
            DrawableSurface { direct_context: RefCell::new(direct_context), layer: self.clone(), surface }
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
        let compositor = &AppGlobals::get().compositor.backend;
        let win32_handle = match window {
            RawWindowHandle::Win32(w) => w,
            _ => panic!("expected a Win32 window handle"),
        };
        let interop = compositor.compositor
            .cast::<ICompositorDesktopInterop>()
            .expect("could not retrieve ICompositorDesktopInterop");
        let desktop_window_target = interop
            .CreateDesktopWindowTarget(HWND(win32_handle.hwnd.get() as *mut c_void), false)
            .expect("could not create DesktopWindowTarget");
        desktop_window_target
            .SetRoot(&self.0.visual)
            .expect("SetRoot failed");
        // self.compositor.
        self.0.window_target.replace(Some(desktop_window_target));
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Compositor impl
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Windows compositor backend
pub(crate) struct Compositor {
    compositor: WinCompositor,
    dxgi_factory: IDXGIFactory3,
    device: ID3D12Device,
    command_queue: ID3D12CommandQueue,
    completion_fence: ID3D12Fence,
    completion_event: Win32Event,
    completion_fence_value: Cell<u64>,
    debug: IDXGIDebug1,
    //composition_graphics_device: CompositionGraphicsDevice,
    //composition_device: IDCompositionDesktopDevice,
    direct_context: RefCell<sk::gpu::DirectContext>,
}

impl Drop for Compositor {
    fn drop(&mut self) {
        self.wait_for_gpu();
    }
}

impl Compositor {
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
        //let resource = unsafe { cp::from_raw(image.into_raw() as *mut Sk_ID3D12Resource) };

        //let mut texture_resource_info = TextureResourceInfo::from_resource(image);
        //texture_resource_info.format = format;

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
        let sk_surface = wrap_backend_render_target(
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

    pub(crate) fn new(app_backend: &backend::AppBackend) -> Compositor {
        let direct_context = unsafe {
            // SAFETY: backend_context is valid I guess?
            sk::gpu::DirectContext::new_d3d(
                &sk::gpu::d3d::BackendContext {
                    adapter: app_backend.adapter.as_ref().expect("no adapter selected").clone(),
                    device: app_backend.d3d12_device.0.clone(),
                    queue: app_backend.d3d12_command_queue.0.clone(),
                    memory_allocator: None,
                    protected_context: Protected::No,
                },
                None,
            )
            .expect("failed to create D3D context")
        };

        let compositor = WinCompositor::new().expect("failed to create compositor");
        let dxgi_factory = app_backend.dxgi_factory.0.clone();
        let device = app_backend.d3d12_device.0.clone();
        let command_queue = app_backend.d3d12_command_queue.0.clone();

        let command_completion_fence = unsafe {
            device
                .CreateFence::<ID3D12Fence>(0, D3D12_FENCE_FLAG_NONE)
                .expect("CreateFence failed")
        };

        let debug = unsafe { DXGIGetDebugInterface1(0).unwrap() };

        let command_completion_event = unsafe {
            let event = CreateEventW(None, false, false, None).unwrap();
            Win32Event::from_raw(event)
        };

        Compositor {
            compositor,
            dxgi_factory,
            device,
            debug,
            command_queue,
            completion_fence: command_completion_fence,
            completion_event: command_completion_event,
            completion_fence_value: Cell::new(0),
            direct_context: RefCell::new(direct_context),
        }
    }

    /// Creates a surface layer.
    ///
    /// FIXME: don't ignore format
    pub(crate) fn create_surface_layer(&self, size: Size, _format: ColorType) -> Layer {
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
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
        };
        let swap_chain: IDXGISwapChain3 = unsafe {
            self.dxgi_factory
                .CreateSwapChainForComposition(&self.command_queue, &swap_chain_desc, None)
                .expect("CreateSwapChainForComposition failed")
                .cast::<IDXGISwapChain3>()
                .unwrap()
        };
        let frame_latency_waitable = unsafe {
            swap_chain.SetMaximumFrameLatency(1).unwrap();
            swap_chain.GetFrameLatencyWaitableObject()
        };

        let swap_chain = SwapChain {
            inner: swap_chain,
            frame_latency_waitable: unsafe { Owned::new(frame_latency_waitable) },
        };

        // Create the composition surface representing the swap chain in the compositor
        let surface = unsafe {
            self.compositor
                .cast::<ICompositorInterop>()
                .unwrap()
                .CreateCompositionSurfaceForSwapChain(&swap_chain.inner)
                .expect("could not create composition surface for swap chain")
        };

        // Create the visual+brush holding the surface
        let visual = self.compositor.CreateSpriteVisual().unwrap();
        let brush = self.compositor.CreateSurfaceBrush().unwrap();
        brush.SetSurface(&surface).unwrap();
        visual.SetBrush(&brush).unwrap();
        let new_size = Vector2::new(size.width as f32, size.height as f32);
        visual.SetSize(new_size).unwrap();

        Layer(Rc::new(LayerInner {
            visual: visual.cast().unwrap(),
            size: Cell::new(size),
            swap_chain: Some(swap_chain),
            window_target: RefCell::new(None),
        }))
    }

    /// Waits for submitted GPU commands to complete.
    fn wait_for_gpu(&self) {
        //let _span = span!("wait_for_gpu_command_completion");
        unsafe {
            let mut val = self.completion_fence_value.get();
            val += 1;
            self.completion_fence_value.set(val);
            self.command_queue
                .Signal(&self.completion_fence, val)
                .expect("ID3D12CommandQueue::Signal failed");
            if self.completion_fence.GetCompletedValue() < val {
                self.completion_fence
                    .SetEventOnCompletion(val, self.completion_event.handle())
                    .expect("SetEventOnCompletion failed");
                WaitForSingleObject(self.completion_event.handle(), 0xFFFFFFFF);
            }
        }
    }


    pub(crate) fn flush_and_present(&self, swap_chain: &IDXGISwapChain3) {
        {
            let _span = span!("skia: flush_and_submit");
            self.direct_context.borrow_mut().flush_and_submit();
        }

        unsafe {
            let _span = span!("D3D12: present");
            swap_chain.Present(1, DXGI_PRESENT::default()).unwrap();

            if let Some(client) = tracy_client::Client::running() {
                client.frame_mark();
            }
        }
    }
}
