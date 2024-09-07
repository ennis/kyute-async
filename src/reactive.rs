use tokio::sync::watch;

/// Observable property.
pub struct Property<T> {
    value: watch::Sender<T>
}


impl<T> Property<T> {

    /// Creates a new property with the given initial value.
    pub fn new(value: T) -> Self {
        let (tx, _) = watch::channel(value);
        Self {
            value: tx
        }
    }

    /// Returns an async stream of the values of the property.
    ///
    /// It firsts yields the current value of the property, and then
    /// yields the new values as they are set.
    pub fn stream(&self) -> watch::Receiver<T> {
        self.value.subscribe()
    }

    pub fn modify(&self, f: impl FnOnce(&mut T) -> bool) -> bool {
        self.value.send_if_modified(f)
    }

    pub fn borrow(&self) -> watch::Ref<T> {
        self.value.borrow()
    }
}

impl<T: Clone> Property<T> {
    /// Returns the current value of the property.
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }
}

/*
impl<T: Eq> Property<T> {
    /// Sets the value of the property.
    pub fn set(&self, value: T) {
        self.value.send_if_modified(value);
    }
}*/

/*
#[cfg(test)]
mod tests {
    use super::*;

    // Q: should stuff like DecoratedBox have all its properties as bindings?
    // A tokio::watch is heavy.
    //
    // A: No. Basic elements like Text & DecoratedBox should not have properties. Only
    //    methods to set the value directly.
    //    Elements using those can then decide how & when they call those methods.
    //    Typically, they would call it inside their async task in response to some event.

    #[observable]
    struct State {
        count: i32,
        text: String,
    }

    struct State(Property<StateInner>);

    impl State {

        // Not going to write this by hand for sure... that's worse than Q_PROPERTY.

        fn count(&self) -> i32 {
            self.0.borrow().count.clone()
        }

        fn set_count(&self, count: i32) {
            self.0.modify(|state| {
                if state.count != count {
                    state.count = count;
                    true
                } else {
                    false
                }
            });
        }
    }

    #[tokio::test]
    async fn test_observable() {


        let state = Property::new(State {
            count: 0,
            text: "Hello".to_string(),
        });

        tokio::task::spawn_local(async move {
            let mut stream = state.stream();

        });

        state.modify(|state| {
            state.count += 1;
            true
        });
    }
}*/