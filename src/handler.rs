use std::{
    cell::{Cell, RefCell},
    future::poll_fn,
    task::{Poll, Waker},
};

use slab::Slab;

struct Listener {
    notified: bool,
    waker: Option<Waker>,
}

struct HandlerInner<T: Clone + ?Sized> {
    listeners: Slab<Listener>,
    sender_waker: Option<Waker>,
    value: Option<T>,
}

/// Event handlers.
pub struct Handler<T: Clone + ?Sized> {
    /// The number of listeners that have been notified, but have not yet processed the value.
    ack_remaining: Cell<usize>,
    inner: RefCell<HandlerInner<T>>,
}

impl<T: Clone> Default for Handler<T>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> Handler<T> {
    pub fn new() -> Self {
        Handler {
            inner: RefCell::new(HandlerInner {
                listeners: Slab::new(),
                sender_waker: None,
                value: None,
            }),
            ack_remaining: Cell::new(0),
        }
    }

    pub async fn emit(&self, value: T) {
        self.ready().await;

        let mut inner = self.inner.borrow_mut();
        // don't bother if there are no listeners
        if inner.listeners.is_empty() {
            return;
        }

        // set the value to propagate
        inner.value.replace(value);
        // set the number of listeners that need to acknowledge the value
        self.ack_remaining.set(inner.listeners.len());

        // wake up all listeners
        for (_, listener) in inner.listeners.iter_mut() {
            if let Some(waker) = listener.waker.take() {
                listener.notified = true;
                waker.wake();
            }
        }

        drop(inner);

        self.ready().await;
    }

    pub async fn ready(&self) {
        poll_fn(|cx| {
            //let mut inner = self.inner.borrow_mut();
            if self.ack_remaining.get() > 0 {
                // not ready
                let mut inner = self.inner.borrow_mut();
                if let Some(ref waker) = inner.sender_waker {
                    if waker.will_wake(cx.waker()) {
                        return Poll::Pending;
                    }
                }
                inner.sender_waker = Some(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
    }


    pub async fn wait(&self) -> T {
        let slot = self.inner.borrow_mut().listeners.insert(Listener {
            notified: false,
            waker: None,
        });

        poll_fn(move |cx| {
            let mut inner = self.inner.borrow_mut();
            if inner.listeners[slot].notified {
                inner.listeners.remove(slot);
                // Acknowledge one listener
                self.ack_remaining.set(self.ack_remaining.get() - 1);
                // If we're the last listener, wake the sender
                if self.ack_remaining.get() == 0 {
                    if let Some(waker) = inner.sender_waker.take() {
                        waker.wake();
                    }
                }

                Poll::Ready(inner.value.clone())
            } else {
                inner.listeners[slot].waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await.unwrap()
    }
}
