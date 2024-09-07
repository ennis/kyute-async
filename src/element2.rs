use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};
use bitflags::bitflags;
use kurbo::Point;
use tokio::sync::mpsc::{channel, Receiver, Sender, unbounded_channel};
use tokio::task::{JoinHandle, spawn_local};
use crate::event::Event;
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


// Store ElementBase within Button?
// Or store Button within Element?

/// A visual element in the UI tree.
pub trait Element : Any {
    /// Returns a reference to the `ElementBase` member.
    fn base(&self) -> &ElementBase;
    fn layout(&self, box_constraints: BoxConstraints) -> Geometry;
    fn hit_test(&self, point: Point) -> bool;

    // Provided methods
    fn mark_layout_dirty(&self) {
        self.base().mark_layout_dirty();
    }

    fn add_child(self: &Rc<Self>, child: ElementPtr) {
        child.remove();
        child.base().parent.replace(Rc::downgrade(self));
        self.base().children.borrow_mut().push(child);
        self.mark_layout_dirty();
    }

    /// Removes this element from the UI tree.
    fn remove(self: &Rc<Self>) {
        if let Some(parent) = self.parent() {
            parent.remove_child(self);
        }
    }

    fn remove(&self) {
        self.base().remove()
    }

    fn remove_child(&self, child: &ElementPtr) {
        self.base().remove_child(child)
    }

    /*pub fn with_handler<F, Fut>(f: F) -> Self where
        Fut: Future<Output=()> + 'static,
        F: FnOnce(WeakElementPtr) -> Fut + 'static
    {
        let element = ElementBase::new();
        element.set_handler(f);
        element
    }*/

    /// Sets the element handler task.
    fn set_handler<F, Fut>(self: &Rc<Self>, f: F) where
        Fut: Future<Output=()> + 'static,
        F: FnOnce(WeakElementPtr) -> Fut + 'static
    {
        // If we just pass a copy of the Rc in the task,
        // it will lead to a reference cycle, because the task will keep the Rc alive.
        // But using a weak reference is annoying because we have to upgrade it
        // every time we access the widget in the task. It's the "right thing" to do, but it's fairly
        // annoying.
        //
        // We also don't want to abort the task when orphaning, since we may reattach it later.
        //
        // We can't rely on refcount=1 to mean that last remaining reference is the one in the task,
        // because the task does not necessarily keep a strong ref.
        //
        // What we want is to abort the task when one reference in particular drops
        // (the one held by the parent task that created the element). But we have no
        // way to know which one it is.

        let (tx_events, rx) = channel(16);
        let handle = ElementHandle {
            element: Rc::downgrade(&self.0),
            events: rx,
        };
        let task = spawn_local(async move {
            f(handle).await;
        });
        self.0.handler.replace(Some(ElementHandler {
            tx_events,
            task
        }));
    }
}

// Purposefully not clonable, since this controls the lifetime of the associated task.
// Still not OK, there might be an element reference inside the tree
pub struct Element<T>(Rc<T>);

impl<T> Drop for Element<T> {
    fn drop(&mut self) {
        // If the element has a handler, we should abort it.
        if let Some(handler) = self.0.handler.borrow_mut().take() {
            handler.tx_events.close();
            handler.task.abort();
        }
    }
}

pub type ElementPtr = Rc<dyn Element>;
pub type WeakElementPtr = Weak<dyn Element>;

pub struct ElementBase {
    /// Parent element.
    parent: RefCell<WeakElementPtr>,
    /// Transform relative to parent.
    transform: Cell<kurbo::Affine>,
    /// Last computed geometry.
    geometry: Cell<Geometry>,
    change_flags: Cell<ChangeFlags>,
    /// Child elements.
    children: RefCell<Vec<ElementPtr>>,
    handler: RefCell<Option<ElementHandler>>,
}

impl ElementBase {
    /// Creates a new element with the default size and transform, and no parent.
    pub fn new() -> Self {
        ElementBase {
            parent: Default::default(),
            transform: Default::default(),
            geometry: Default::default(),
            change_flags: Default::default(),
            children: Default::default(),
            handler: Default::default(),
            //visual: RefCell::new(Rc::new(NullVisual)),
        }
    }



    /*
    /// Sets the visual delegate of the element.
    ///
    /// Returns the previous visual delegate.
    pub fn set_visual(&self, visual: Rc<impl VisualDelegate + 'static>) -> Rc<dyn VisualDelegate> {
        self.0.visual.replace(visual)
    }*/

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
    pub fn parent(&self) -> Option<ElementPtr> {
        self.parent.borrow().clone().upgrade()
    }

    /// Returns the last computed geometry of this element.
    pub fn geometry(&self) -> Geometry {
        self.geometry.get()
    }

    /// Returns the transform that converts from local to parent coordinates.
    pub fn transform(&self) -> kurbo::Affine {
        self.transform.get()
    }

    /// Sets the transform that converts from local to parent coordinates.
    ///
    /// This should be called by `VisualDelegate` during `layout`
    pub fn set_transform(&self, transform: kurbo::Affine) {
        self.transform.set(transform);
    }

    /// Marks the layout of the element as dirty.
    pub fn mark_layout_dirty(&self) {
        self.change_flags.set(self.change_flags.get().union(ChangeFlags::LAYOUT));
        // TODO: only recurse if the flag wasn't set before
        // recursively mark the parent hierarchy as dirty
        if let Some(parent) = self.parent() {
            parent.mark_layout_dirty();
        }
    }

    /*
    /// Adds a child element and sets its parent to this element.
    // FIXME: can't implement this, we don't have Rc<Self>
    pub fn add_child(&self, child: Element) {
        child.remove();
        child.0.parent.replace(WeakElement(Rc::downgrade(&self.0)));
        self.0.children.borrow_mut().push(child);
        self.mark_layout_dirty();
    }*/

    /// Removes all child elements.
    pub fn clear_children(&self) {
        self.children.borrow_mut().clear();
        self.mark_layout_dirty();
    }

    /// Removes the specified element from the children.
    pub fn remove_child(&self, child: &ElementPtr) {
        let index = self.children.borrow().iter().position(|c| Rc::ptr_eq(&c, &child));
        if let Some(index) = index {
            self.children.borrow_mut().remove(index);
            self.mark_layout_dirty();
        }
    }

}

//impl<T: Element + ?Sized>

/*
/// Weak reference to an element.
#[derive(Clone, Default)]
pub struct WeakElement(Weak<ElementInner>);

impl WeakElement {
    /// Attempts to upgrade the weak reference to a strong reference.
    pub fn upgrade(&self) -> Option<Element> {
        self.0.upgrade().map(Element)
    }
}*/


struct ElementHandler {
    /// Tx end of the channel to send events.
    tx_events: Sender<Event>,
    task: JoinHandle<()>
}


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
}


/// Delegate for the layout, rendering and hit-testing of an element.
pub trait VisualDelegate: Any {
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


// Styling: how do padding, colors of visual elements get their values?
//
// Are they dynamic bindings?
// -> No
//
// When the style changes (somehow), how is the UI tree updated?
// -> rebuild the whole UI tree when an environment changes
//
// Each element has an associated "Environment"; when modified, it signals all children to update (via an event, or a separate tx channel).
// When an environment changes, send event to all children to update.

//
// One TX channel per element? is that too much?




// Observable<Rc<Style>>
// -> Observable<Insets>