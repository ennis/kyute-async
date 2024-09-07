//use crate::app_globals::AppGlobals;
use anyhow::Context;
use futures::{
    channel::mpsc::{Receiver, Sender},
    executor,
    executor::{LocalPool, LocalSpawner},
    future::{abortable, AbortHandle},
    task::{ArcWake, LocalSpawnExt},
};
use futures_util::SinkExt;
use scoped_tls::scoped_thread_local;
use smallvec::SmallVec;
use std::{cell::RefCell, collections::HashMap, fmt, future::Future, rc::Rc, sync::Arc, time::Instant};
use std::sync::OnceLock;
use tracing::warn;
use tracy_client::set_thread_name;
use winit::{
    event::Event,
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopWindowTarget},
    window::WindowId,
};
use winit::event_loop::EventLoopProxy;

/// Event loop user event.
#[derive(Clone, Debug)]
pub enum ExtEvent {
    /// Triggers an UI update
    UpdateUi,
}

static EVENT_LOOP_PROXY: OnceLock<EventLoopProxy<ExtEvent>> = OnceLock::new();

pub fn wake_event_loop() {
    EVENT_LOOP_PROXY.get().unwrap().send_event(ExtEvent::UpdateUi).unwrap()
}


scoped_thread_local!(static EVENT_LOOP_WINDOW_TARGET: EventLoopWindowTarget<ExtEvent>);

/// Accesses the current "event loop window target", which is used to create winit [winit::window::Window]s.
pub fn with_event_loop_window_target<T>(f: impl FnOnce(&EventLoopWindowTarget<ExtEvent>) -> T) -> T {
    EVENT_LOOP_WINDOW_TARGET.with(|event_loop| f(&event_loop))
}

struct AppState {
    windows: RefCell<HashMap<WindowId, Sender<winit::event::WindowEvent>>>,
    spawner: LocalSpawner,
}

scoped_thread_local!(static APP_STATE: AppState);

/// Spawns a task on the main-thread executor.
pub fn spawn(fut: impl Future<Output = ()> + 'static) -> AbortHandle {
    APP_STATE.with(|state| {
        let (fut, abort_handle) = abortable(fut);
        state
            .spawner
            .spawn_local(async {
                let _ = fut.await; // ignore aborts
            })
            .expect("failed to spawn task");
        abort_handle
    })
}

/// Registers a winit window with the application, and retrieves the events for the window.
///
/// # Return value
///
/// An async receiver used to receive events for this window.
pub fn register_window(window_id: WindowId) -> Receiver<winit::event::WindowEvent> {
    let (tx, rx) = futures::channel::mpsc::channel(16);
    APP_STATE.with(|state| {
        state.windows.borrow_mut().insert(window_id, tx);
    });
    rx
}

pub fn quit() {
    with_event_loop_window_target(|event_loop| {
        event_loop.exit();
    });
}

pub fn run(root_future: impl Future<Output = ()> + 'static) -> Result<(), anyhow::Error> {
    set_thread_name!("UI thread");
    let event_loop: EventLoop<ExtEvent> = EventLoopBuilder::with_user_event()
        .build()
        .context("failed to create the event loop")?;

    EVENT_LOOP_PROXY.set(event_loop.create_proxy()).expect("run was called twice");

    //AppGlobals::new();

    event_loop.set_control_flow(ControlFlow::Wait);
    let _event_loop_start_time = Instant::now();

    let mut local_pool = LocalPool::new();
    let app_state = AppState {
        windows: RefCell::new(HashMap::new()),
        spawner: local_pool.spawner(),
    };

    let result = APP_STATE.set(&app_state, || {
        // Before the event loop starts, spawn the root future, and poll it
        // so that the initial windows are created.
        // This is necessary because if no windows are created no messages will be sent and
        // the closure passed to `run` will never be called.
        EVENT_LOOP_WINDOW_TARGET.set(&event_loop, || {
            spawn(root_future);
            local_pool.run_until_stalled();
        });

        event_loop.run(move |event, elwt| {
            EVENT_LOOP_WINDOW_TARGET.set(elwt, || {
                //let event_time = Instant::now().duration_since(event_loop_start_time);

                match event {
                    Event::WindowEvent {
                        window_id,
                        event: window_event,
                    } => {
                        APP_STATE.with(|state| {
                            // retrieve the channel endpoint to send events to the window
                            if let Some(mut sender) = state.windows.borrow_mut().get_mut(&window_id) {
                                // this will unblock tasks waiting for events
                                let _ = sender.start_send(window_event);
                            }
                        });
                    }
                    _ => {}
                };

                // run tasks that were possibly unblocked as a result of propagating events
                local_pool.run_until_stalled();
            });
        })?;
        Ok(())
    });

    result
}
