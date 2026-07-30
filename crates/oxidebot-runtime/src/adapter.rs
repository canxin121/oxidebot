use crate::{AdapterError, BotDescriptor, BotServices};
use async_trait::async_trait;

#[path = "adapter_frame.rs"]
mod adapter_frame;
#[path = "adapter_ingress.rs"]
mod adapter_ingress;
#[path = "adapter_interest.rs"]
mod adapter_interest;

pub use adapter_frame::{
    EventBatchFrame, EventFrame, FrameIndex, InboundFrame, MessageFrame, MessageFrameBuilder,
};
pub use adapter_ingress::{AdapterContext, Submission};
pub(crate) use adapter_ingress::{AdapterLimits, EventSink, IngressBatch};
pub use adapter_interest::InterestPlan;

/// Whether normal completion is meaningful for an adapter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AdapterMode {
    /// Long-lived network transport. Returning before cancellation is fatal.
    #[default]
    Persistent,
    /// Finite replay/import adapter that may finish successfully.
    Finite,
}

/// One platform transport and its outbound services.
#[async_trait]
pub trait Adapter: Send + 'static {
    /// Describes the bot identity and platform represented by this adapter.
    fn descriptor(&self) -> BotDescriptor;
    /// Returns the outbound API services implemented by this adapter.
    fn services(&self) -> BotServices;
    /// Returns whether normal completion is valid for this transport.
    fn mode(&self) -> AdapterMode {
        AdapterMode::Persistent
    }
    /// Runs inbound transport processing until shutdown or a terminal adapter error.
    async fn run(self: Box<Self>, context: AdapterContext)
        -> std::result::Result<(), AdapterError>;
}

#[cfg(test)]
mod retained_size_tests {
    use super::*;
    use oxidebot_core::{
        event::{EventType, MessageEvent},
        source::{
            message::Message,
            user::{User, UserProfile},
        },
        BotSlot, CompactId, ConversationRef, Event, EventId, PlatformId,
    };

    #[test]
    fn message_frame_charges_owned_profile_strings() {
        let id = EventId::new("retained-profile").expect("static event id");
        let baseline = MessageFrame::text(
            id.clone(),
            ConversationRef::direct("room"),
            "user",
            "1",
            "hello",
        );
        let baseline_bytes = baseline.retained_bytes();
        let frame = MessageFrame::text(id, ConversationRef::direct("room"), "user", "1", "hello")
            .sender(User {
                id: "user".into(),
                profile: Some(UserProfile {
                    display_name: Some("x".repeat(1024 * 1024)),
                    ..UserProfile::default()
                }),
            });

        assert!(frame.retained_bytes() >= baseline_bytes.saturating_add(1024 * 1024));
    }

    #[test]
    fn canonical_event_frame_derives_message_routing_metadata() {
        let event = Event::Message(MessageEvent {
            id: "message-1".into(),
            time: None,
            sender: User::new("alice"),
            conversation: ConversationRef::group("room"),
            message: Message::text("/ping one"),
        });
        let frame = EventFrame::new(EventId::new("event-1").expect("static id"), event);
        let platform = PlatformId::new("test").expect("static platform");
        let index = frame
            .index(BotSlot(0), &platform)
            .expect("canonical event indexes");
        let event = index.events.first().expect("one event index");

        assert_eq!(event.event_type, EventType::Message);
        assert_eq!(event.command.as_deref(), Some("ping"));
        assert_eq!(
            event.conversation.as_ref().map(|key| &key.id),
            Some(&CompactId::from("room"))
        );
        assert_eq!(
            event.actor.as_ref().map(|key| &key.id),
            Some(&CompactId::from("alice"))
        );
    }

    #[test]
    fn canonical_event_frame_derives_interaction_and_native_keys() {
        let platform = PlatformId::new("test").expect("static platform");
        let interaction = Event::Interaction(oxidebot_core::interaction::InteractionEvent {
            id: "interaction-1".into(),
            kind: oxidebot_core::interaction::InteractionKind::Button,
            action_id: Some("approve".into()),
            values: Vec::new(),
            user: User::new("alice"),
            conversation: None,
            message: None,
            context_id: None,
            response: None,
            fields: Default::default(),
            command: None,
            locale: None,
            permissions: Default::default(),
            data: serde_json::Value::Null,
        });
        let interaction = EventFrame::new(
            EventId::new("event-interaction").expect("static id"),
            interaction,
        )
        .index(BotSlot(0), &platform)
        .expect("interaction indexes");
        assert_eq!(
            interaction.events[0].interaction.as_deref(),
            Some("approve")
        );
    }
}
