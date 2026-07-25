use std::any::Any;

pub use crate::interaction::InteractionEvent;
use any::AnyEvent;
pub use lifecycle::{EventEnvelope, LifecycleEvent};
pub use message::MessageEvent;
pub use meta::MetaEvent;
pub use notice::NoticeEvent;
pub use request::RequestEvent;

pub mod any;
pub mod lifecycle;
pub mod message;
pub mod meta;
pub mod notice;
pub mod request;

#[allow(clippy::large_enum_variant)] // Boxing variants would break the public event API.
#[derive(Debug, Clone)]
pub enum Event {
    MessageEvent(MessageEvent),
    NoticeEvent(NoticeEvent),
    RequestEvent(RequestEvent),
    InteractionEvent(InteractionEvent),
    LifecycleEvent(LifecycleEvent),
    MetaEvent(MetaEvent),
    AnyEvent(AnyEvent),
}

/// EventTrait is a trait that represents the event that the bot triggers.
/// TraitObject can't take self:`Arc<Self>`, so you should impl Send and Sync And Clone(costless clone) for you event
/// Tip: use `Arc` to wrap the your event.
pub trait EventTrait: Send + Sync + Any {
    fn get_events(&self) -> Vec<Event>;

    /// Returns events with portable delivery metadata. Existing adapters get a
    /// default envelope and can override this to expose stable event IDs,
    /// timestamps, retry attempts, raw payloads, and conversation context.
    fn get_event_envelopes(&self) -> Vec<EventEnvelope> {
        self.get_events()
            .into_iter()
            .map(|event| EventEnvelope::new(self.server(), event))
            .collect()
    }
    fn server(&self) -> &'static str;
    // TraitObject can't inherit Clone, so you should manually implement it
    fn clone_box(&self) -> EventObject;
    // TraitObject can't downcast to the concrete type, so you should implement it manually
    fn as_any(&self) -> &dyn Any;
}

pub type EventObject = Box<dyn EventTrait>;

impl Clone for EventObject {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for EventObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let event = self.get_events();
        write!(f, "{:?}", event)
    }
}
