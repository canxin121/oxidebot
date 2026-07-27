use oxidebot_core::EventEnvelope;

/// Fast synchronous admission check applied before route futures are created.
pub trait Filter<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn accepts(&self, event: &EventEnvelope, state: &S) -> bool;
}

impl<S, F> Filter<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(&EventEnvelope, &S) -> bool + Send + Sync + 'static,
{
    fn accepts(&self, event: &EventEnvelope, state: &S) -> bool {
        self(event, state)
    }
}
