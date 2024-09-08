use std::cell::Cell;
use crate::element::{Element, Visual};
use crate::event::Event;
use crate::handler::Handler;
use crate::layout::{BoxConstraints, Geometry};
use kurbo::Point;
use std::rc::{Rc, Weak};

pub struct Button {
    pub element: Element,     // Element base
    pub clicked: Handler<()>, // async handler
    pressed: Cell<bool>,
}

impl Button {
    pub fn new() -> Rc<Button> {
        let mut button = Element::new_derived(|element| {
            Button {
                element,
                clicked: Handler::new(),
                pressed: Cell::new(false),
            }
        });

        //let rect = Rectangle::new();        // Rc<Rectangle>, with Rectangle: impl Visual
        // add_child: Rc<dyn Visual>
        //button.add_child(&rect);    // add_child provided by impl visual, defers to element
        button
    }

    pub async fn clicked(&self) {
        self.clicked.wait().await
    }
}

impl Visual for Button {
    fn element(&self) -> &Element {
        &self.element
    }

    fn layout(&self, constraints: &BoxConstraints) -> Geometry {
        Geometry::default()
    }

    fn hit_test(&self, position: Point) -> bool {
        // check if the position is within the bounds of the button
        false
    }

    async fn event(&self, event: &Event) {
        match event {
            Event::PointerDown(_) => {
                eprintln!("pointer down") ;
                self.pressed.set(true)
            }
            Event::PointerUp(_) => {
                if self.pressed.get() {
                    self.pressed.set(false);
                    eprintln!("pointer up") ;
                    self.clicked.emit(()).await;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_stream() {
        let mut button = Button::new();
        button.clicked().await;
        eprintln!("clicked");
    }
}
