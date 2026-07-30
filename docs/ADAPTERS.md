# Adapter authoring

An OxideBot adapter has three jobs:

1. Normalize platform input into the public `Event` hierarchy.
2. Expose outbound operations through one `CallApiTrait` implementation.
3. Run its transport loop until cancellation (or declare itself `Finite`).

The normal inbound API is intentionally small. For an already-normalized
portable event, call `context.submit_event(event_id, event).await`. The runtime
derives its routing index: message commands are parsed from the canonical
message text, interactions use `action_id`, native events use `kind`, and
conversation/actor keys are derived whenever the public event contains them.
This means a normal adapter never constructs `DispatchIndex`, `DispatchDraft`,
or `DispatchBatch`.

```rust,no_run
use async_trait::async_trait;
use oxidebot::adapter::prelude::*;
use oxidebot::{BotId, Event, PlatformId};
use oxidebot_runtime::{BotDescriptor, BotServices};

struct WebhookAdapter;

#[async_trait]
impl Adapter for WebhookAdapter {
    fn descriptor(&self) -> BotDescriptor {
        BotDescriptor::new(PlatformId::new("acme").unwrap(), BotId::new("bot").unwrap())
    }

    fn services(&self) -> BotServices {
        BotServices::new(std::sync::Arc::new(AcmeApi))
    }

    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        while let Some((id, event)) = receive_and_normalize().await? {
            context.submit_event(id, event).await?;
        }
        Ok(())
    }
}

struct AcmeApi;
impl oxidebot::CallApiTrait for AcmeApi {}

async fn receive_and_normalize() -> Result<Option<(oxidebot::EventId, Event)>, AdapterError> {
    Ok(None)
}
```

Use `MessageFrame` only when the incoming payload is a canonical message but
you want the named sender/conversation builder. Use `InboundFrame` only for a
high-throughput transport that can produce an inexpensive pre-decode routing
index and reuse parsing work in `decode_indexed`.

`CallApiTrait::plan_outgoing_message` receives the `MessageTarget` as well as
the message. This is deliberate: platform limits and permissions often differ
between direct conversations, groups, threads, and interaction contexts. Keep
bot-wide immutable feature support in `bot_capabilities`; return a
target-specific plan from the planner when necessary.

An adapter is `Persistent` by default. A file importer, replay fixture, or
bounded webhook batch must return `AdapterMode::Finite`; only finite adapters
can be used with `run_to_completion`.

## Identity types at the adapter boundary

Do not flatten platform identities into unrelated `String` values. The
canonical model distinguishes `UserId`, `ConversationId`, `MessageId`, and
`RoleId`; each is a transparent wrapper around a lossless `CompactId`, so a
numeric Telegram ID stays numeric when serialized. This makes accidental use
of a user ID as a message ID a compile-time error.

Construct an ID from a trusted adapter value with `Into` and let the canonical
event fields determine its type:

```rust,no_run
use oxidebot::{Message, MessageId, UserId};
use oxidebot::core::{ConversationRef, User};

let sender = User::new(UserId::from(42_u64));
let conversation = ConversationRef::direct_user(sender.id.clone());
let mut message = Message::text("hello");
message.id = Some(MessageId::from(7_u64));
```

`Message::id` is `Option<MessageId>`: a freshly authored outgoing message has
no platform ID until delivery succeeds. For a handler extractor, use
`IncomingMessageId`; the facade-level `MessageId` is the canonical identity
value used by outbound APIs and message references. Command `Mention` values
and template mentions likewise carry `UserId`, rather than a bare user-ID
string.
