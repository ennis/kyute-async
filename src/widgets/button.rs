use futures::Stream;
use futures_util::SinkExt;
use futures_util::stream::StreamExt;
use tokio::select;
use crate::element::{Element, ElementInner};
use crate::handler::Handler;
use crate::layout::BoxConstraints;

pub struct Button {
    pub element: Element,   // Element base
    pub clicked: Handler<()>,   // async handler
}

impl Button {
    pub fn new() -> Button {
        let mut button = Button {
            element: Element::new(),
            clicked: Handler::new(),
        };

        let rect = Rectangle::new();        // Rc<Rectangle>, with Rectangle: impl Visual
        // add_child: Rc<dyn Visual>
        button.add_child(&rect);    // add_child provided by impl visual, defers to element
        button
    }
}

impl Visual for Button {
    fn base(&self) -> &Element {
        &self.element
    }

    fn layout(&self, constraints: &BoxConstraints) -> Layout {
        Layout::default()
    }
}

async fn button_handler(this: &ElementInner<Button>) {

    // Rectangle is a simple Rc<> wrapper and implements the Element trait
    // It contains no futures. There's

    let rect = Rectangle::new();    // Rc<Rectangle>

    // set style...
    // woops, can't set parent on rect, no Rc<> available for this
    this.set_content(&rect);

    let mut pressed = false;
    loop {
        // listen for mouse events
        select! {
            _ = rect.mouse_down() => {
                pressed = true;
            }
            _ = rect.mouse_up() => {
                if pressed {
                    pressed = false;
                    this.clicked.emit(()).await;
                }
            }
        }
    }
}

pub fn button() -> Element<Button> {
    Element::with_future(button_handler)
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