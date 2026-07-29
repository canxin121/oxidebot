use oxidebot_core::Event;

/// Fast synchronous admission check over the single public event model.
pub trait Filter<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Returns whether this handler filter accepts `event` under `state`.
    fn accepts(&self, event: &Event, state: &S) -> bool;
}

impl<S, F> Filter<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(&Event, &S) -> bool + Send + Sync + 'static,
{
    fn accepts(&self, event: &Event, state: &S) -> bool {
        self(event, state)
    }
}
