use std::any::Any;

use any::AnyEvent;
use anyhow::Result;
pub use message::MessageEvent;
pub use meta::MetaEvent;
pub use notice::NoticeEvent;
pub use request::RequestEvent;

pub mod any;
pub mod message;
pub mod meta;
pub mod notice;
pub mod request;

#[derive(Debug, Clone)]
pub enum Event {
    MessageEvent(MessageEvent),
    NoticeEvent(NoticeEvent),
    RequestEvent(RequestEvent),
    MetaEvent(MetaEvent),
    AnyEvent(AnyEvent),
}

/// TraitObject can't take self:Arc<Self>, so you should impl Send and Sync And Clone for you event
/// Tip: use `Arc` to wrap the your event.
pub trait EventTrait: Send + Sync + Any {
    fn into_event(&self) -> Result<Event>;
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
        let event = self.into_event();
        write!(f, "{:?}", event)
    }
}
