//use crate::app_globals::AppGlobals;
use crate::app_globals::AppGlobals;
use crate::element::Visual;
use anyhow::Context;
use futures::channel::mpsc::{Receiver, Sender};
use futures::executor::{LocalPool, LocalSpawner};
use futures::future::{abortable, AbortHandle};
use futures::task::{ArcWake, LocalSpawnExt};
use futures_util::future::LocalBoxFuture;
use futures_util::{FutureExt, SinkExt};
use scoped_tls::scoped_thread_local;
use smallvec::SmallVec;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::future::{poll_fn, Future};
use std::rc::{Rc, Weak};
use std::sync::{Arc, OnceLock};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};
use tracing::warn;
use tracy_client::set_thread_name;
use windows::System::IUserDeviceAssociationChangedEventArgs;
use winit::event::{Event, StartCause};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use winit::window::WindowId;

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

struct Timer {
    waker: Waker,
    deadline: Instant,
}

struct AppState {
    windows: RefCell<HashMap<WindowId, Weak<dyn WindowHandler>>>,
    spawner: LocalSpawner,
    timers: RefCell<SmallVec<Timer, 4>>,
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

pub trait WindowHandlerObjectSafe {
    fn event_future<'a>(&'a self, event: &'a winit::event::WindowEvent) -> LocalBoxFuture<'a, ()>;
}

/// Handler for window events.
pub trait WindowHandler: WindowHandlerObjectSafe {
    async fn event(&self, event: &winit::event::WindowEvent)
    where
        Self: Sized;
}

impl<T: WindowHandler> WindowHandlerObjectSafe for T
where
    T: WindowHandler,
{
    fn event_future<'a>(&'a self, event: &'a winit::event::WindowEvent) -> LocalBoxFuture<'a, ()> {
        self.event(event).boxed_local()
    }
}

/// Registers a winit window with the application, and retrieves the events for the window.
///
/// # Return value
///
/// An async receiver used to receive events for this window.
pub fn register_window(window_id: WindowId, handler: Rc<dyn WindowHandler>) {
    APP_STATE.with(|state| {
        state.windows.borrow_mut().insert(window_id, Rc::downgrade(&handler));
    });
}

pub fn quit() {
    with_event_loop_window_target(|event_loop| {
        event_loop.exit();
    });
}

pub async fn wait_until(deadline: Instant) {
    let mut registered = false;
    poll_fn(move |cx| {
        APP_STATE.with(|state| {
            if Instant::now() >= deadline {
                return Poll::Ready(());
            } else if !registered {
                // set waker
                let timers = &mut *state.timers.borrow_mut();
                timers.push(Timer {
                    waker: cx.waker().clone(),
                    deadline,
                });
                registered = true;
            }
            Poll::Pending
        })
    })
    .await
}

/// Waits for the specified duration.
pub async fn wait_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    wait_until(deadline).await;
}

pub fn run(root_future: impl Future<Output = ()> + 'static) -> Result<(), anyhow::Error> {
    set_thread_name!("UI thread");
    let event_loop: EventLoop<ExtEvent> = EventLoopBuilder::with_user_event()
        .build()
        .context("failed to create the event loop")?;

    EVENT_LOOP_PROXY
        .set(event_loop.create_proxy())
        .expect("run was called twice");

    AppGlobals::new();

    event_loop.set_control_flow(ControlFlow::Wait);
    let _event_loop_start_time = Instant::now();

    let mut local_pool = LocalPool::new();
    let app_state = AppState {
        windows: RefCell::new(HashMap::new()),
        spawner: local_pool.spawner(),
        timers: RefCell::new(Default::default()),
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
                APP_STATE.with(|state| {
                    match event {
                        Event::NewEvents(cause) => {
                            match cause {
                                StartCause::ResumeTimeReached { .. }
                                | StartCause::WaitCancelled { .. }
                                | StartCause::Poll => {
                                    // wake all expired timers
                                    let timers = &mut *state.timers.borrow_mut();
                                    let now = Instant::now();
                                    while let Some(timer) = timers.first() {
                                        if timer.deadline <= now {
                                            let timer = timers.remove(0);
                                            timer.waker.wake();
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                StartCause::Init => {}
                            }
                        }
                        Event::WindowEvent {
                            window_id,
                            event: window_event,
                        } => {
                            eprintln!("[{:?}] [{:?}]", window_id, window_event);
                            // Don't hold a borrow of `state.windows` across the handler since
                            // the handler may create new windows.
                            let handler = state.windows.borrow().get(&window_id).cloned();
                            if let Some(handler) = handler {
                                if let Some(handler) = handler.upgrade() {
                                    local_pool.run_until(handler.event_future(&window_event));
                                } else {
                                    // remove the window if the handler has been dropped
                                    state.windows.borrow_mut().remove(&window_id);
                                }
                            }
                        }
                        _ => {}
                    };

                    // run tasks that were possibly unblocked as a result of propagating events
                    local_pool.run_until_stalled();

                    // set control flow to wait until next timer expires, or wait until next
                    // event if there are no timers
                    let timers = &mut **state.timers.borrow_mut();
                    if !timers.is_empty() {
                        timers.sort_by_key(|t| t.deadline);
                        elwt.set_control_flow(ControlFlow::WaitUntil(timers[0].deadline));
                    } else {
                        elwt.set_control_flow(ControlFlow::Wait);
                    }
                });
            });
        })?;
        Ok(())
    });

    AppGlobals::teardown();
    result
}
