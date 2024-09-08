use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::{pending, Future};
use std::marker::PhantomPinned;
use std::ops::Deref;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, OnceLock};
use std::{mem, ptr, task};

use bitflags::bitflags;
use futures_util::future::LocalBoxFuture;
use futures_util::task::ArcWake;
use futures_util::FutureExt;
use kurbo::Point;
use pin_weak::rc::PinWeak;
use slab::Slab;

use crate::{application, PaintCtx};
use crate::event::Event;
use crate::handler::Handler;
use crate::layout::{BoxConstraints, Geometry};

bitflags! {
    #[derive(Copy, Clone, Default)]
    pub struct ChangeFlags: u32 {
        /// The transform of the element has changed.
        const TRANSFORM = 0b0001;
        /// The size of the element has changed.
        const SIZE = 0b0010;
        /// The layout of the element needs to be recalculated.
        const LAYOUT = 0b0100;
        /// The element needs to be rendered.
        const RENDER = 0b1000;
        const NONE = 0b0000;
    }
}


pub type AnyVisual = Rc<dyn Visual>;

/// The inner state of an element.
pub struct Element {
    _pin: PhantomPinned,
    weak_this: Weak<dyn Visual>,
    key: Cell<usize>,
    parent: RefCell<Option<Weak<dyn Visual>>>,
    transform: Cell<kurbo::Affine>,
    geometry: Cell<Geometry>,
    change_flags: Cell<ChangeFlags>,
    children: RefCell<Vec<AnyVisual>>,
    // self-referential
    // would be nice if we didn't have to allocate
    // would be nice if this was a regular task
    // NOTE: we already allocate for the VisualDelegate, we might as well allocate another for the
    // shared state between the task and the element, instead of this weird self-reference thing.
    // It's not like we're allocating on every event.
    //future: RefCell<Option<LocalBoxFuture<'static, ()>>>,
    //state: T,
}

impl Element {
    pub fn new(weak_this: &Weak<dyn Visual>) -> Element {
        Element {
            _pin: PhantomPinned,
            weak_this: weak_this.clone(),
            key: Cell::new(0),
            parent: RefCell::new(None),
            transform: Cell::new(kurbo::Affine::default()),
            geometry: Cell::new(Geometry::default()),
            change_flags: Cell::new(ChangeFlags::NONE),
            children: RefCell::new(Vec::new()),
        }
    }

    pub fn new_derived<'a, T: Visual + 'static>(f: impl FnOnce(Element) -> T) -> Rc<T> {
        Rc::new_cyclic(move |weak: &Weak<T>| {
            let weak : Weak<dyn Visual> = weak.clone();
            let element = Element::new(&weak);
            let visual = f(element);
            visual
        })
    }
}

impl Drop for Element {
    fn drop(&mut self) {
    }
}

pub trait Visual: EventTarget {
    fn element(&self) -> &Element;
    fn layout(&self, constraints: &BoxConstraints) -> Geometry;
    fn hit_test(&self, point: Point) -> bool;
    fn paint(&self, ctx: &mut PaintCtx) {}

    // Why async? this is because the visual may transfer control to async event handlers
    // before returning.
    async fn event(&self, event: &Event) where Self: Sized;
}

trait EventTarget {
    fn event_future<'a>(&'a self, event: &'a Event) -> LocalBoxFuture<'a, ()>;
}

impl<W> EventTarget for W where W: Visual {
    fn event_future<'a>(&'a self, event: &'a Event) -> LocalBoxFuture<'a, ()> {
        self.event(event).boxed_local()
    }
}

impl dyn Visual + '_ {
    /// Returns this visual as a reference-counted pointer.
    pub fn rc(&self) -> Rc<dyn Visual> {
        self.element().weak_this.upgrade().unwrap()
    }

    /// Adds a child visual and sets its parent to this visual.
    pub fn add_child(&self, child: &dyn Visual) {
        let this = self.element();
        child.remove();
        child.element().parent.replace(Some(this.weak_this.clone()));
        this.children.borrow_mut().push(child.rc());
        //this.mark_layout_dirty();
    }

    /// Removes all child visuals.
    pub fn clear_children(&self) {
        self.element().children.borrow_mut().clear();
        //self.mark_layout_dirty();
    }

    /// Removes the specified visual from the children of this visual.
    pub fn remove_child(&self, child: &dyn Visual) {
        let this = self.element();
        let index = this.children.borrow().iter().position(|c| ptr::eq(&**c, child));

        if let Some(index) = index {
            this.children.borrow_mut().remove(index);
            //self.mark_layout_dirty();
        }
    }

    /// Returns the parent of this visual, if it has one.
    pub fn parent(&self) -> Option<Rc<dyn Visual>> {
        self.element().parent.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Removes this visual from its parent.
    pub fn remove(&self) {
        if let Some(parent) = self.parent() {
            parent.remove_child(self);
        }
    }

    pub async fn send_event(&self, event: &Event) {
        // issue: allocating on every event is not great
        self.event_future(event).await;
    }
}

/*
/// Async fns that take an element as argument (arguments to `with_future`).
pub trait ElementFn<'a, T> {
    type Future: Future + 'a;
    fn call(self, source: &'a ElementInner<T>) -> Self::Future;
}

impl<'a, T: 'static, F, R> ElementFn<'a, T> for F
where
    F: FnOnce(&'a ElementInner<T>) -> R,
    R: Future + 'a,
{
    type Future = F::Output;
    fn call(self, source: &'a ElementInner<T>) -> Self::Future {
        self(source)
    }
}*/

// A visual element in the UI tree.
// NOTE: it's not a wrapper type because we want unsized coercion to work (and CoerceUnsized is not stable).
// The ergonomics around unsized coercion is really atrocious currently.
//#[derive(Clone)]
//pub struct Element<T: 'static + ?Sized = dyn Any>(Pin<Rc<ElementInner<T>>>);

/*
// Manual unsized coercion
impl<T: 'static> From<Element<T>> for Element {
    fn from(value: Element<T>) -> Self {
        Element(value.0)
    }
}

impl<T: 'static + ?Sized> Deref for Element<T> {
    type Target = ElementInner<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}*/

/*
thread_local! {
    static ELEMENT_BY_KEY: RefCell<Slab<WeakElement>> = RefCell::new(Slab::new());
}

static ELEMENT_WAKEUP_QUEUE: OnceLock<Mutex<VecDeque<usize>>> = OnceLock::new();

pub fn wakeup_element(key: usize) {
    let mut queue = ELEMENT_WAKEUP_QUEUE
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap();
    queue.push_back(key);
}


#[derive(Clone)]
struct ElementWaker {
    key: usize,
}

impl ArcWake for ElementWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        wakeup_element(arc_self.key);
        application::wake_event_loop();
    }
}

impl Element {
    fn poll(&self) {
        let future = &mut *self.0.future.borrow_mut();
        if let Some(future) = future.as_mut() {
            let waker = futures_util::task::waker(Arc::new(ElementWaker { key: self.key.get() }));
            let cx = &mut task::Context::from_waker(&waker);
            let _ = future.as_mut().poll(cx);
        }
    }
}

/// Element executor
pub(crate) fn poll_elements() {
    let mut queue = {
        let queue = &mut *ELEMENT_WAKEUP_QUEUE
            .get_or_init(|| Mutex::new(VecDeque::new()))
            .lock()
            .unwrap();
        mem::take(queue)
    };

    while let Some(key) = queue.pop_front() {
        if let Some(element) = ELEMENT_BY_KEY.with_borrow(|elements| elements.get(key).and_then(WeakElement::upgrade)) {
            element.poll()
        }
    }
}*/

/*
impl Element {
    /// Sets the visual delegate of the element.
    ///
    /// Returns the previous visual delegate.
    pub fn set_visual(&self, visual: Rc<impl VisualDelegate + 'static>) -> Rc<dyn VisualDelegate> {
        self.0.visual.replace(visual)
    }

    pub fn weak(&self) -> WeakElement {
        WeakElement(PinWeak::downgrade(self.0.clone()))
    }

    /// Modifies the visual delegate of the element.
    ///
    /// This doesn't mark the element as dirty, so you should call `mark_layout_dirty`
    /// after if needed.
    ///
    /// # Panics
    ///
    /// Panics if the visual delegate is not of the specified type.
    pub fn modify_visual<V: VisualDelegate + 'static>(&self, f: impl FnOnce(&mut Rc<V>)) {
        // TODO
    }


    /// Returns the last computed geometry of this element.
    pub fn geometry(&self) -> Geometry {
        self.0.geometry.get()
    }

    /// Returns the transform that converts from local to parent coordinates.
    pub fn transform(&self) -> kurbo::Affine {
        self.0.transform.get()
    }

    /// Sets the transform that converts from local to parent coordinates.
    ///
    /// This should be called by `VisualDelegate` during `layout`
    pub fn set_transform(&self, transform: kurbo::Affine) {
        self.0.transform.set(transform);
    }

    /// Marks the layout of the element as dirty.
    pub fn mark_layout_dirty(&self) {
        self.0
            .change_flags
            .set(self.0.change_flags.get().union(ChangeFlags::LAYOUT));
        // TODO: only recurse if the flag wasn't set before
        // recursively mark the parent hierarchy as dirty
        if let Some(parent) = self.parent() {
            parent.mark_layout_dirty();
        }
    }
}*/

/*
/// Weak reference to an element.
#[derive(Clone)]
pub struct WeakElement(PinWeak<ElementInner<dyn Any>>);

impl WeakElement {
    /// Attempts to upgrade the weak reference to a strong reference.
    pub fn upgrade(&self) -> Option<Element<dyn Any>> {
        self.0.upgrade().map(Element)
    }
}*/

/*
/// A handle to an element in the UI tree that can be used to set its content,
/// and receive events asynchronously.
pub struct ElementHandle {
    element: WeakElement,
    events: Receiver<Event>,
}

impl ElementHandle {
    /// Sets the content of the element.
    pub fn set_content(&self, content: Element) {
        if let Some(element) = self.element.upgrade() {
            element.clear_children();
            element.add_child(content);
        }
    }

    /// Removes the element from the tree.
    pub fn remove(&self) {
        if let Some(element) = self.element.upgrade() {
            element.remove();
        }
    }

    /// Waits for the next event.
    pub async fn next_event(&mut self) -> Event {
        self.events.recv().await.unwrap()
    }
}*/

