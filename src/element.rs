use std::{mem, ptr, task};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::{Future, pending};
use std::marker::PhantomPinned;
use std::ops::Deref;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use bitflags::bitflags;
use futures_util::future::LocalBoxFuture;
use futures_util::FutureExt;
use futures_util::task::ArcWake;
use kurbo::Point;
use pin_weak::rc::PinWeak;
use slab::Slab;

use crate::application;
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

// We are allocating for:
// - the VisualDelegate
// - the future (BoxFuture)
// - the ElementInner itself
// - the Vec of children
// - the Event handler

/// The inner state of an element.
pub struct ElementInner<T: ?Sized + 'static> {
    _pin: PhantomPinned,
    key: Cell<usize>,
    parent: RefCell<Option<WeakElement>>,
    transform: Cell<kurbo::Affine>,
    geometry: Cell<Geometry>,
    change_flags: Cell<ChangeFlags>,
    events: Handler<Event>,
    children: RefCell<Vec<Element<dyn Any>>>,
    visual: RefCell<Rc<dyn VisualDelegate>>,
    // self-referential
    // would be nice if we didn't have to allocate
    // would be nice if this was a regular task
    // NOTE: we already allocate for the VisualDelegate, we might as well allocate another for the
    // shared state between the task and the element, instead of this weird self-reference thing.
    // It's not like we're allocating on every event.
    future: RefCell<Option<LocalBoxFuture<'static, ()>>>,
    state: T,
}

// Move the future out of ElementInner? Use a special element instead?

impl<T: 'static + ?Sized> Drop for ElementInner<T> {
    fn drop(&mut self) {
        // FIXME: this is problematic because we have a &mut self here,
        // and at the same time, a &self in the future. If somehow the future has a drop impl
        // that accesses the element, this breaks everything.
        //
        // Actually, miri doesn't complain until we explicitly make a `&mut ref` to a field.


        ELEMENT_BY_KEY.with_borrow_mut(|elements| {
            elements.remove(self.key.get());
        });
    }
}

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
}

/// A visual element in the UI tree.
// NOTE: it's not a wrapper type because we want unsized coercion to work (and CoerceUnsized is not stable).
// The ergonomics around unsized coercion is really atrocious currently.
#[derive(Clone)]
pub struct Element<T: 'static + ?Sized = dyn Any>(Pin<Rc<ElementInner<T>>>);

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
}

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

impl<T: 'static + Default> Element<T> {
    /// Creates a new element with the default size and transform, and no parent.
    pub fn new() -> Self {
        // Rc<Pin> is there to make sure we don't call Rc::try_unwrap
        // or do anything that would move ElementInner.
        // Otherwise, Rc already guarantees that the pointee won't move in memory.
        let element = Element(Rc::pin(ElementInner {
            _pin: Default::default(),
            parent: Default::default(),
            key: Default::default(),
            transform: Default::default(),
            geometry: Default::default(),
            change_flags: Default::default(),
            events: Handler::new(),
            children: Default::default(),
            visual: RefCell::new(Rc::new(NullVisual)),
            future: RefCell::new(None),
            state: T::default(),
        }));
        ELEMENT_BY_KEY.with_borrow_mut(|elements| {
            let key = elements.insert(WeakElement(PinWeak::downgrade(element.0.clone())));
            element.key.set(key);
        });
        element
    }

    pub fn with_future<F>(f: F) -> Element<T>
    where
        F: for<'a> ElementFn<'a, T> + 'static,
    {
        let mut element = Element::new();
        let ptr = &*element.0 as *const ElementInner<T>;

        let future = async move {
            // SAFETY:
            // - the pointee is pinned, so it can't move
            // - the future is dropped before the pointee so the future won't outlive the element
            let inner = unsafe { &*ptr };
            f.call(inner).await;
            // Element futures should never return; we always drop them as the element is dropped.
            pending::<()>().await;
        };

        element.0.future.borrow_mut().replace(future.boxed_local());

        {
            let element_any: Element = element.clone().into();
            element_any.poll();
        }

        element
    }
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
}

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

    /// Returns the parent element, if any.
    pub fn parent(&self) -> Option<Element<dyn Any>> {
        self.0.parent.borrow().as_ref().and_then(WeakElement::upgrade)
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

    /// Adds a child element and sets its parent to this element.
    pub fn add_child(&self, child: Element) {
        child.remove();
        child
            .0
            .parent
            .replace(Some(WeakElement(PinWeak::downgrade(self.0.clone()))));
        self.0.children.borrow_mut().push(child);
        self.mark_layout_dirty();
    }

    /// Removes all child elements.
    pub fn clear_children(&self) {
        self.0.children.borrow_mut().clear();
        self.mark_layout_dirty();
    }

    /// Removes the specified element from the children.
    pub fn remove_child(&self, child: &Element) {
        let index = self
            .0
            .children
            .borrow()
            .iter()
            .position(|c| ptr::eq(&*c.0.as_ref(), &*child.0.as_ref()));
        if let Some(index) = index {
            self.0.children.borrow_mut().remove(index);
            self.mark_layout_dirty();
        }
    }

    /// Removes this element from the UI tree.
    pub fn remove(&self) {
        if let Some(parent) = self.parent() {
            parent.remove_child(self);
        }
    }
}

/// Weak reference to an element.
#[derive(Clone)]
pub struct WeakElement(PinWeak<ElementInner<dyn Any>>);

impl WeakElement {
    /// Attempts to upgrade the weak reference to a strong reference.
    pub fn upgrade(&self) -> Option<Element<dyn Any>> {
        self.0.upgrade().map(Element)
    }
}

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

/// Delegate for the layout, rendering and hit-testing of an element.
pub trait VisualDelegate: Any {
    fn layout(&self, this_element: &Element, children: &[Element], box_constraints: BoxConstraints) -> Geometry;
    fn hit_test(&self, this_element: &Element, point: Point) -> Option<Element>;
}

pub struct NullVisual;

impl VisualDelegate for NullVisual {
    fn layout(&self, _this_element: &Element, _children: &[Element], _box_constraints: BoxConstraints) -> Geometry {
        Geometry::default()
    }

    fn hit_test(&self, _this_element: &Element, _point: Point) -> Option<Element> {
        None
    }
}
