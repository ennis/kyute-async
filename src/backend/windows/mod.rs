//! Windows implementation details
use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::mem;
use std::ops::Deref;
use std::rc::Rc;
use std::time::Duration;

use skia_safe::gpu::Protected;
use threadbound::ThreadBound;
use windows::core::{IUnknown, Interface, Owned};
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandQueue, ID3D12Device, ID3D12Fence,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_FENCE_FLAG_NONE,
};
use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGIGetDebugInterface1, IDXGIAdapter1, IDXGIDebug1, IDXGIFactory3, DXGI_ADAPTER_DESC1,
    DXGI_CREATE_FACTORY_FLAGS,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::UI::Composition::Compositor;

pub(crate) use compositor::{DrawableSurface, Layer};
mod compositor;

/////////////////////////////////////////////////////////////////////////////
// COM wrappers
/////////////////////////////////////////////////////////////////////////////

// COM thread safety notes: some interfaces are thread-safe, some are not, and for some we don't know due to poor documentation.
// Additionally, some interfaces should only be called on the thread in which they were created.
//
// - For thread-safe interfaces: wrap them in a `Send+Sync` newtype
// - For interfaces bound to a thread: wrap them in `ThreadBound`
// - For interfaces not bound to a thread but with unsynchronized method calls:
//      wrap them in a `Send` newtype, and if you actually need to call the methods from multiple threads, `Mutex`.

/// Defines a send+sync wrapper over a windows interface type.
///
/// This signifies that it's OK to call the interface's methods from multiple threads simultaneously:
/// the object itself should synchronize the calls.
macro_rules! sync_com_ptr_wrapper {
    ($wrapper:ident ( $iface:ident ) ) => {
        #[derive(Clone)]
        pub(crate) struct $wrapper(pub(crate) $iface);
        unsafe impl Sync for $wrapper {} // ok to send &I across threads
        unsafe impl Send for $wrapper {} // ok to send I across threads
        impl ::std::ops::Deref for $wrapper {
            type Target = $iface;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

/*/// Defines a send wrapper over a windows interface type.
///
/// This signifies that it's OK to call an interface's methods from a different thread than that in which it was created.
/// However, you still have to synchronize the method calls yourself (with, e.g., a `Mutex`).
macro_rules! send_com_ptr_wrapper {
    ($wrapper:ident ( $iface:ident ) ) => {
        #[derive(Clone)]
        pub(crate) struct $wrapper(pub(crate) $iface);
        unsafe impl Send for $wrapper {} // ok to send I across threads
        impl ::std::ops::Deref for $wrapper {
            type Target = $iface;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}*/

sync_com_ptr_wrapper! { D3D12Device(ID3D12Device) }
sync_com_ptr_wrapper! { DXGIFactory3(IDXGIFactory3) }
sync_com_ptr_wrapper! { D3D12CommandQueue(ID3D12CommandQueue) }
sync_com_ptr_wrapper! { DWriteFactory(IDWriteFactory) }
sync_com_ptr_wrapper! { D3D12Fence(ID3D12Fence) }
//sync_com_ptr_wrapper! { D3D11Device(ID3D11Device5) }
//sync_com_ptr_wrapper! { WICImagingFactory2(IWICImagingFactory2) }
//sync_com_ptr_wrapper! { D2D1Factory1(ID2D1Factory1) }
//sync_com_ptr_wrapper! { D2D1Device(ID2D1Device) }
//send_com_ptr_wrapper! { D2D1DeviceContext(ID2D1DeviceContext) }

/////////////////////////////////////////////////////////////////////////////
// AppBackend
/////////////////////////////////////////////////////////////////////////////

struct GpuFenceData {
    fence: ID3D12Fence,
    event: Owned<HANDLE>,
    value: Cell<u64>,
}

struct BackendInner {
    pub(crate) dispatcher_queue_controller: DispatcherQueueController,
    pub(crate) adapter: IDXGIAdapter1,
    pub(crate) d3d12_device: D3D12Device,              // thread safe
    pub(crate) command_queue: D3D12CommandQueue, // thread safe
    pub(crate) command_allocator: ThreadBound<ID3D12CommandAllocator>,
    pub(crate) dxgi_factory: DXGIFactory3,
    pub(crate) dwrite_factory: DWriteFactory,
    /// Fence data used to synchronize GPU and CPU (see `wait_for_gpu`).
    sync: GpuFenceData,
    /// Windows compositor instance (Windows.UI.Composition).
    compositor: Compositor,
    debug: IDXGIDebug1,
    direct_context: RefCell<skia_safe::gpu::DirectContext>,
    //composition_graphics_device: CompositionGraphicsDevice,
    //composition_device: IDCompositionDesktopDevice,
}


impl BackendInner {
    /// Waits for submitted GPU commands to complete.
    fn wait_for_gpu(&self) {
        //let _span = span!("wait_for_gpu_command_completion");
        unsafe {
            let mut val = self.sync.value.get();
            val += 1;
            self.sync.value.set(val);
            self.command_queue
                .Signal(&self.sync.fence, val)
                .expect("ID3D12CommandQueue::Signal failed");
            if self.sync.fence.GetCompletedValue() < val {
                self.sync
                    .fence
                    .SetEventOnCompletion(val, *self.sync.event)
                    .expect("SetEventOnCompletion failed");
                WaitForSingleObject(*self.sync.event, 0xFFFFFFFF);
            }
        }
    }
}

#[derive(Clone)]
pub struct ApplicationBackend(Rc<BackendInner>);

impl Drop for ApplicationBackend {
    fn drop(&mut self) {
        // Synchronize with the GPU when dropping the backend.
        self.0.wait_for_gpu();
    }
}

impl ApplicationBackend {
    pub(crate) fn new() -> ApplicationBackend {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap() };

        // Dispatcher queue
        // SAFETY: FFI
        let dispatcher_queue_controller = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })
            .expect("failed to create dispatcher queue controller")
        };

        // DirectWrite factory
        let dwrite_factory = unsafe {
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            DWriteFactory(dwrite)
        };

        //=========================================================
        // DXGI Factory and adapter enumeration

        // SAFETY: the paramters are valid
        let dxgi_factory =
            unsafe { DXGIFactory3(CreateDXGIFactory2::<IDXGIFactory3>(DXGI_CREATE_FACTORY_FLAGS::default()).unwrap()) };

        // --- Enumerate adapters
        let mut adapters = Vec::new();
        unsafe {
            let mut i = 0;
            while let Ok(adapter) = dxgi_factory.EnumAdapters1(i) {
                adapters.push(adapter);
                i += 1;
            }
        };

        let mut chosen_adapter = None;
        for adapter in adapters.iter() {
            let desc = unsafe { adapter.GetDesc1().unwrap() };

            use std::os::windows::ffi::OsStringExt;

            let name = &desc.Description[..];
            let name_len = name.iter().take_while(|&&c| c != 0).count();
            let name = OsString::from_wide(&desc.Description[..name_len])
                .to_string_lossy()
                .into_owned();
            tracing::info!(
                "DXGI adapter: name={}, LUID={:08x}{:08x}",
                name,
                desc.AdapterLuid.HighPart,
                desc.AdapterLuid.LowPart,
            );
            /*if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0) != 0 {
                continue;
            }*/
            if chosen_adapter.is_none() {
                chosen_adapter = Some(adapter.clone())
            }
        }
        let adapter = chosen_adapter.expect("no suitable video adapter found");

        //=========================================================
        // D3D12 stuff

        let debug = unsafe { DXGIGetDebugInterface1(0).unwrap() };

        let d3d12_device = unsafe {
            let mut d3d12_device: Option<ID3D12Device> = None;
            D3D12CreateDevice(
                // pAdapter:
                &adapter.cast::<IUnknown>().unwrap(),
                // MinimumFeatureLevel:
                D3D_FEATURE_LEVEL_12_0,
                // ppDevice:
                &mut d3d12_device,
            )
            .expect("D3D12CreateDevice failed");
            D3D12Device(d3d12_device.unwrap())
        };

        let command_queue = unsafe {
            let cqdesc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0,
                Flags: Default::default(),
                NodeMask: 0,
            };
            let cq: ID3D12CommandQueue = d3d12_device
                .0
                .CreateCommandQueue(&cqdesc)
                .expect("CreateCommandQueue failed");
            D3D12CommandQueue(cq)
        };

        let command_allocator = unsafe {
            let command_allocator = d3d12_device
                .0
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .unwrap();
            ThreadBound::new(command_allocator)
        };

        //=========================================================
        // Compositor

        let direct_context = unsafe {
            // SAFETY: backend_context is valid I guess?
            skia_safe::gpu::DirectContext::new_d3d(
                &skia_safe::gpu::d3d::BackendContext {
                    adapter: adapter.clone(),
                    device: d3d12_device.0.clone(),
                    queue: command_queue.0.clone(),
                    memory_allocator: None,
                    protected_context: Protected::No,
                },
                None,
            )
            .expect("failed to create skia context")
        };

        let compositor = Compositor::new().expect("failed to create compositor");

        let sync = {
            let fence = unsafe {
                d3d12_device
                    .CreateFence::<ID3D12Fence>(0, D3D12_FENCE_FLAG_NONE)
                    .expect("CreateFence failed")
            };
            let event = unsafe { Owned::new(CreateEventW(None, false, false, None).unwrap()) };

            GpuFenceData {
                fence,
                event,
                value: Cell::new(0),
            }
        };

        ApplicationBackend(Rc::new(BackendInner {
            d3d12_device,
            command_queue,
            command_allocator,
            dxgi_factory,
            dwrite_factory,
            dispatcher_queue_controller,
            adapter,
            compositor,
            sync,
            debug,
            direct_context: RefCell::new(direct_context),
        }))
    }


    /// Returns the system double click time in milliseconds.
    pub(crate) fn double_click_time(&self) -> Duration {
        unsafe {
            let ms = GetDoubleClickTime();
            Duration::from_millis(ms as u64)
        }
    }
}
