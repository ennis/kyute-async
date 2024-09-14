//!
use std::cell::Cell;
use std::rc::Rc;
use crate::element::{AnyVisual, Element, Visual};
use crate::event::Event;
use crate::handler::Handler;

#[derive(Copy,Clone,Debug, Default)]
pub struct InteractState {
    pub active: bool,
    pub hovered: bool,
    pub focused: bool,
}

pub struct Interact {
    pub element: Element,
    pub clicked: Handler<()>,
    pub hovered: Handler<bool>,
    pub active: Handler<bool>,
    pub focused: Handler<bool>,
    pub state_changed: Handler<InteractState>,
    state: Cell<InteractState>,
}

impl Interact {
    pub fn new() -> Rc<Interact> {
        let elem =  Element::new_derived(|element| Interact {
            element,
            clicked: Handler::new(),
            hovered: Handler::new(),
            active: Handler::new(),
            focused: Handler::new(),
            state_changed: Handler::new(),
            state: Default::default()
        });
        elem
    }

    async fn update_state(&self, state: InteractState) {
        self.state.set(state);
        self.state_changed.emit(state).await;
    }

    pub async fn state_changed(&self) -> InteractState {
        self.state_changed.wait().await
    }

    pub async fn clicked(&self) {
        self.clicked.wait().await
    }
}

impl Visual for Interact {
    fn element(&self) -> &Element {
        &self.element
    }


    async fn event(&self, event: &mut Event)
    where
        Self: Sized
    {
        let mut state = self.state.get();
        match event {
            Event::PointerDown(_) => {
                state.active = true;
                self.update_state(state).await;
                self.active.emit(true).await;
            }
            Event::PointerUp(_) => {
                if state.active {
                    state.active = false;
                    self.update_state(state).await;
                    self.clicked.emit(()).await;
                }
            }
            Event::PointerEnter(_) => {
                state.hovered = true;
                self.update_state(state).await;
                self.hovered.emit(true).await;
            }
            Event::PointerLeave(_) => {
                state.hovered = false;
                self.update_state(state).await;
                self.hovered.emit(false).await;
            }
            _ => {}
        }
    }
}