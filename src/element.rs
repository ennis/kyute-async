use std::any::{Any, TypeId};
use std::cell::{Cell, Ref, RefCell};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::marker::PhantomPinned;
use std::ops::Deref;
use std::ptr;
use std::ptr::addr_eq;
use std::rc::{Rc, Weak};

use bitflags::bitflags;
use futures_util::future::LocalBoxFuture;
use futures_util::FutureExt;
use kurbo::{Affine, Point, Vec2};
use crate::compositor::DrawableSurface;

use crate::event::Event;
use crate::layout::{BoxConstraints, Geometry};
use crate::PaintCtx;

bitflags! {
    #[derive(Copy, Clone, Default)]
    pub struct ChangeFlags: u32 {
        const PAINT = 0b0001;
        const LAYOUT = 0b0010;
        const NONE = 0b0000;
    }
}

pub trait AttachedProperty: Any {
    type Value: Clone;

    fn set(self, item: &dyn Visual, value: Self::Value) where Self: Sized {
        item.set::<Self>(value);
    }

    fn get(self, item: &dyn Visual) -> Option<Self::Value> where Self: Sized {
        item.get::<Self>()
    }
}


/// Wrapper over Rc<dyn Visual> that has PartialEq impl.
#[derive(Clone)]
#[repr(transparent)]
pub struct AnyVisual(pub(crate) Rc<dyn Visual>);

impl PartialOrd for AnyVisual {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AnyVisual {
    fn cmp(&self, other: &Self) -> Ordering {
        Rc::as_ptr(&self.0).cast::<()>().cmp(&Rc::as_ptr(&other.0).cast::<()>())
    }
}

impl Eq for AnyVisual {}

impl From<Rc<dyn Visual>> for AnyVisual {
    fn from(rc: Rc<dyn Visual>) -> Self {
        AnyVisual(rc)
    }
}

impl Deref for AnyVisual {
    type Target = dyn Visual;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl PartialEq for AnyVisual {
    fn eq(&self, other: &Self) -> bool {
        self.0.is_same(&*other.0)
    }
}

/// Base state of an element.
pub struct Element {
    _pin: PhantomPinned,
    /// Weak pointer to this element.
    weak_this: Weak<dyn Visual>,
    /// TODO unused
    key: Cell<usize>,
    /// This element's parent.
    parent: RefCell<Option<Weak<dyn Visual>>>,
    /// Layout: transform from local to parent coordinates.
    transform: Cell<kurbo::Affine>,
    /// Layout: geometry (size and baseline) of this element.
    geometry: Cell<Geometry>,
    /// TODO unused
    change_flags: Cell<ChangeFlags>,
    /// List of child elements.
    children: RefCell<Vec<AnyVisual>>,
    /// Name of the element.
    name: RefCell<String>,

    attached_properties: RefCell<BTreeMap<TypeId, Box<dyn Any>>>,
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
            change_flags: Cell::new(ChangeFlags::LAYOUT | ChangeFlags::PAINT),
            children: RefCell::new(Vec::new()),
            name: RefCell::new(format!("{:p}", weak_this.as_ptr())),
            attached_properties: Default::default(),
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

    pub fn children(&self) -> Ref<[AnyVisual]> {
        Ref::map(self.children.borrow(), |v| v.as_slice())
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry.get()
    }

    /// Adds a child visual and sets its parent to this visual.
    pub fn add_child(&self, child: &dyn Visual) {
        child.remove();
        child.element().parent.replace(Some(self.weak_this.clone()));
        self.children.borrow_mut().push(child.element().rc().into());
        //this.mark_layout_dirty();
    }

    /// Removes all child visuals.
    pub fn clear_children(&self) {
        self.children.borrow_mut().clear();
        //self.mark_layout_dirty();
    }

    /// Removes the specified visual from the children of this visual.
    pub fn remove_child(&self, child: &dyn Visual) {
        let index = self.children.borrow().iter().position(|c| ptr::eq(&**c, child));

        if let Some(index) = index {
            self.children.borrow_mut().remove(index);
            //self.mark_layout_dirty();
        }
    }

    /// Returns the parent of this visual, if it has one.
    pub fn parent(&self) -> Option<Rc<dyn Visual>> {
        self.parent.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Removes this visual from its parent.
    pub fn remove(&self) {
        if let Some(parent) = self.parent() {
            let self_child = self.weak_this.as_ptr();
            let index = self.children.borrow().iter().position(|c| ptr::eq(&**c, self_child));

            if let Some(index) = index {
                self.children.borrow_mut().remove(index);
                //self.mark_layout_dirty();
            }
        }
    }

    /// Returns the transform of this visual relative to its parent.
    ///
    /// Shorthand for `self.element().transform.get()`.
    pub fn transform(&self) -> Affine {
        self.transform.get()
    }

    pub fn set_transform(&self, transform: Affine) {
        self.transform.set(transform);
    }

    pub fn set_offset(&self, offset: Vec2) {
        self.set_transform(Affine::translate(offset));
    }

    /// Returns the transform from this visual's coordinate space to the coordinate space of the parent window.
    ///
    /// This walks up the parent chain and multiplies the transforms, so consider reusing the result instead
    /// of calling this function multiple times.
    pub fn window_transform(&self) -> Affine {
        let mut transform = self.transform();
        let mut parent = self.parent();
        while let Some(p) = parent {
            transform *= p.transform();
            parent = p.parent();
        }
        transform
    }

    /// Returns the list of ancestors of this visual, plus this visual itself, sorted from the root
    /// to this visual.
    pub fn ancestors_and_self(&self) -> Vec<Rc<dyn Visual>> {
        let mut ancestors = Vec::new();
        let mut current = self.rc();
        while let Some(parent) = current.parent() {
            ancestors.push(parent.clone());
            current = parent;
        }
        ancestors.reverse();
        ancestors.push(self.rc());
        ancestors
    }


    /// Returns this visual as a reference-counted pointer.
    pub fn rc(&self) -> Rc<dyn Visual + 'static> {
        self.weak_this.upgrade().unwrap()
    }

    fn set_dirty_flags(&self, flags: ChangeFlags) {
        let flags = self.change_flags.get() | flags;
        self.change_flags.set(flags);
        if let Some(parent) = self.parent() {
            parent.set_dirty_flags(flags);
        }
    }

    pub fn mark_needs_repaint(&self) {
        self.set_dirty_flags(ChangeFlags::PAINT);
    }

    pub fn mark_needs_relayout(&self) {
        self.set_dirty_flags(ChangeFlags::LAYOUT | ChangeFlags::PAINT);
    }

    pub(crate) fn mark_layout_done(&self) {
        self.change_flags.set(self.change_flags.get() & !ChangeFlags::LAYOUT);
    }

    pub(crate) fn mark_paint_done(&self) {
        self.change_flags.set(self.change_flags.get() & !ChangeFlags::PAINT);
    }

    pub fn needs_relayout(&self) -> bool {
        self.change_flags.get().contains(ChangeFlags::LAYOUT)
    }

    pub fn needs_repaint(&self) -> bool {
        self.change_flags.get().contains(ChangeFlags::PAINT)
    }
}

/// Nodes in the visual tree.
pub trait Visual: EventTarget {
    fn element(&self) -> &Element;
    fn layout(&self, children: &[AnyVisual], constraints: &BoxConstraints) -> Geometry {
        // The default implementation just returns the union of the geometry of the children.
        let mut geometry = Geometry::default();
        for child in children {
            let child_geometry = child.do_layout(constraints);
            geometry.size.width = geometry.size.width.max(child_geometry.size.width);
            geometry.size.height = geometry.size.height.max(child_geometry.size.height);
            geometry.bounding_rect = geometry.bounding_rect.union(child_geometry.bounding_rect);
            geometry.paint_bounding_rect = geometry.paint_bounding_rect.union(child_geometry.paint_bounding_rect);
            child.set_offset(Vec2::ZERO);
        }
        geometry
    }
    fn hit_test(&self, point: Point) -> bool {
        self.element().geometry.get().size.to_rect().contains(point)
    }
    fn paint(&self, ctx: &mut PaintCtx) {}

    // Why async? this is because the visual may transfer control to async event handlers
    // before returning.
    async fn event(&self, event: &mut Event) where Self: Sized {}
}

/// Implementation detail of `Visual` to get an object-safe version of `async fn event()`.
trait EventTarget {
    fn event_future<'a>(&'a self, event: &'a mut Event) -> LocalBoxFuture<'a, ()>;
}

impl<T> EventTarget for T
where T: Visual {
    fn event_future<'a>(&'a self, event: &'a mut Event) -> LocalBoxFuture<'a, ()> {
        self.event(event).boxed_local()
    }
}

/// An entry in the hit-test chain that leads to the visual that was hit.
#[derive(Clone)]
pub struct HitTestEntry {
    /// The visual in the chain.
    pub visual: Rc<dyn Visual>,
    // Transform from the visual's CS to the CS of the visual on which `do_hit_test` was called (usually the root visual of the window).
    //pub root_transform: Affine,
}

impl PartialEq for HitTestEntry {
    fn eq(&self, other: &Self) -> bool {
        self.visual.is_same(&*other.visual)
    }
}

impl Eq for HitTestEntry {}

impl<'a> Deref for dyn Visual + 'a {
    type Target = Element;

    fn deref(&self) -> &Self::Target {
        self.element()
    }
}

impl dyn Visual + '_ {

    /// Returns the name of the visual.
    pub fn name(&self) -> String {
        self.element().name.borrow().clone()
    }

    pub fn children(&self) -> Ref<[AnyVisual]> {
        self.element().children()
    }

    pub fn set_name(&self, name: impl Into<String>)  {
        self.element().name.replace(name.into());
    }

    /// Identity comparison.
    pub fn is_same(&self, other: &dyn Visual) -> bool {
        // It's probably OK to compare the addresses directly since they should be allocated with
        // Rcs, which always allocates even with ZSTs.
        addr_eq(self, other)
    }

    /// Returns the number of children of this visual.
    pub fn child_count(&self) -> usize {
        self.element().children.borrow().len()
    }

    /// Sets the value of an attached property.
    pub fn set<T: AttachedProperty>(&self, value: T::Value) {
        self.element().attached_properties.borrow_mut().insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Gets the value of an attached property.
    pub fn get<T: AttachedProperty>(&self) -> Option<T::Value> {
        self.element().attached_properties.borrow().get(&TypeId::of::<T>()).map(|v| {
            v.downcast_ref::<T::Value>().expect("invalid type of attached property").clone()
        })
    }

    pub fn do_layout(&self, constraints: &BoxConstraints) -> Geometry {
        let children = self.children();
        let geometry = self.layout(&*children, constraints);
        self.geometry.set(geometry);
        self.mark_layout_done();
        geometry
    }


    /// Iterates over the list of children of this visual.
    ///
    /// Stops iterating if the closure returns `false`.
    pub fn traverse_children(&self, mut f: impl FnMut(&dyn Visual) -> bool) {
        let children = self.children.borrow();
        for child in children.iter() {
            if !f(&**child) {
                break;
            }
        }
    }

    pub async fn send_event(&self, event: &mut Event) {
        // issue: allocating on every event is not great
        self.event_future(event).await;
    }

    /// Hit-tests this visual and its children.
    pub(crate) fn do_hit_test(&self, point: Point) -> Vec<AnyVisual> {
        // Helper function to recursively hit-test the children of a visual.
        // point: point in the local coordinate space of the visual
        // transform: accumulated transform from the local coord space of `visual` to the root coord space
        fn hit_test_rec(visual: &dyn Visual, point: Point, transform: Affine, result: &mut Vec<AnyVisual>) -> bool {
            let mut hit = false;
            // hit-test ourselves
            if visual.hit_test(point) {
                hit = true;
                result.push(visual.rc().into());
            }

            visual.traverse_children(|child| {
                let transform = transform * child.transform();
                let local_point = transform.inverse() * point;
                if hit_test_rec(&*child, local_point, transform, result) {
                    hit = true;
                    false
                } else {
                    true
                }
            });
            hit
        }

        let mut path = Vec::new();
        hit_test_rec(self, point, self.transform(), &mut path);
        path
    }


    pub fn do_paint(&self, surface: &DrawableSurface, scale_factor: f64) {
        let mut paint_ctx = PaintCtx {
            scale_factor,
            window_transform: Default::default(),
            surface,
        };

        // Recursively paint the UI tree.
        fn paint_rec(visual: &dyn Visual, ctx: &mut PaintCtx) {
            visual.paint(ctx);
            for child in visual.children().iter() {
                ctx.with_transform(&child.transform(), |ctx| {
                    // TODO clipping
                    paint_rec(&**child, ctx);
                    child.mark_paint_done();
                });
            }
        }

        paint_rec(self, &mut paint_ctx);
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
