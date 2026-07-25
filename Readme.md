# Oxidebot

**Oxidebot** is a lightweight yet powerful chatbot framework based on Rust and the Tokio runtime. It aims to provide developers with a flexible and extensible environment for bot development through modular design.

## Available Bots
- [onebot_v11_oxidebot](https://github.com/canxin121/onebot_v11_oxidebot)
- [telegram_bot_oxidebot](https://github.com/canxin121/telegram_bot_oxidebot)

## Available Handlers
- [china_unicom_oxidebot](https://github.com/canxin121/china_unicom_oxidebot)

## Example Usage
- [oxidebot_example](https://github.com/canxin121/oxidebot_example)

## Core Concepts

### Bot
`Bot` is the core component of the framework, responsible for providing `Event`s and offering basic API methods for developers to call. It serves as the bridge between the framework and external platforms (such as QQ, Telegram, etc.).

The common API is complemented by `PlatformApiRequest`. Every adapter can expose its complete native API through the same lossless JSON request, response, and multipart-file abstraction. This second layer deliberately retains platform-only capabilities even when no honest cross-platform semantic model exists. Unsupported adapters return a downcastable `UnsupportedPlatformApiError` by default, while callers can inspect `platform_api_methods` or `supports_platform_api_method` before calling a platform-only feature.

### Portable chat-platform model

The v2 common model covers the capabilities shared by Telegram, Discord,
Slack, Microsoft Teams, Lark/Feishu, LINE, QQ and similar bot platforms while
retaining the original 0.1 API:

- `ConversationRef` addresses direct chats, groups, channels, forums, threads
  and topics. A nested thread or topic carries its parent conversation, so an
  adapter never has to encode a hierarchy into an opaque string.
- `MessageRef` and `MessageTarget` are portable message addresses. They are
  used consistently by send, edit, delete, bulk delete, reply, forward, copy,
  pin, poll, checklist and reaction APIs.
- `OutgoingMessage` separates ordered content from message-wide options.
  Content includes plain/rich text, all common media forms, galleries,
  location, contacts, stickers, custom emoji, polls, quizzes, checklists and
  rich layouts.
- `RichText` uses byte-indexed spans with styles such as links, mentions,
  custom emoji, code, quotes, date/time, references and mathematical
  expressions. Adapters validate and convert offsets to their platform's unit
  (Telegram and Discord commonly use UTF-16).
- `RichLayout` represents paragraphs, sections, containers, columns, grids,
  lists, tables, quotations, code, maps, details, media and action rows. It can
  map to Telegram Rich Message, Slack Block Kit, Teams Adaptive Cards, LINE
  Flex Message, Discord embeds/components or a platform-native fallback.
- `MessageOptions` carries reply/quote data, components, notification policy,
  visibility, link-preview and mention policy, protected content, immediate /
  scheduled / draft delivery, idempotency keys, client IDs and metadata.

The message lifecycle is exposed through `CallApiTrait`: send/edit/delete,
bulk delete, reactions, pins, activity indicators, read state, polls,
checklists, topics/threads, profiles, members, permissions, member tags,
moderation, join requests, invite links, history, forwarding/copying and batch
delivery. Application APIs cover localized structured commands, suggestions /
autocomplete, persistent surfaces, Mini Apps and bot profiles. Commerce APIs
cover invoices, shipping, checkout and refunds.

```rust,no_run
use anyhow::Result;
use oxidebot::{
    BotTrait, ConversationRef, MessageContent, MessageOptions, MessageTarget,
    OutgoingMessage, RichText, TextStyle,
};

async fn send_portable_message(bot: &dyn BotTrait) -> Result<()> {
    let text = RichText::plain("Read the docs")
        .span(0..13, TextStyle::Bold)
        .span(5..13, TextStyle::Link {
            url: "https://example.com/docs".to_owned(),
        });
    let message = OutgoingMessage::new([MessageContent::RichText(text)])
        .options(MessageOptions::default().protect_content(true));

    bot.send_outgoing_message(
        MessageTarget::new(ConversationRef::direct("user-id")),
        message,
    )
    .await?;
    Ok(())
}
```

`BotCapabilities` reports support as `Native`, `Emulated` or `Unsupported`
for individual content, delivery, component, interaction, conversation,
collaboration and application features. It also reports hard limits such as
text length, caption length, media-group size, command count and poll-option
count. Callers should use capabilities for user-interface decisions and still
handle the structured `UnsupportedFeatureError`: capabilities can depend on
chat type, bot permissions or account configuration.

Legacy adapters remain valid because every new trait method has a default.
Basic v2 messages are converted to `MessageSegment` when lossless conversion
is possible. Rich content, nested targets or advanced options return an
explicit error instead of being silently discarded.

### Interactive messages and menus

Oxidebot models the complete interaction lifecycle instead of treating a button as message content:

- `MessageOptions` attaches an inline keyboard, reply keyboard, forced reply, or platform-native component tree to a message;
- `InteractionEvent` carries button callbacks, selections, commands, and form submissions;
- `InteractionResponse` acknowledges an event, shows a notification, opens a URL or modal, sends a response, or updates the original message;
- `BotCommandSet` and `ChatMenu` configure command discovery and persistent menu entry points;
- `InteractionCapabilities` lets an adapter report which high-level surfaces it implements, and unsupported operations return a downcastable `UnsupportedInteractionError`.

Components intentionally live in `MessageOptions`, not `MessageSegment`. A segment is ordered message content that can be split or grouped during delivery, while a component tree belongs to the final platform message as a whole.

```rust,no_run
use anyhow::Result;
use oxidebot::{
    api::payload::SendMessageTarget, ActionRow, BotTrait, Button, ButtonStyle, CallApiTrait,
    InlineKeyboard, MessageComponents, MessageOptions,
};
use oxidebot::source::message::MessageSegment;

async fn send_confirmation(bot: &dyn BotTrait) -> Result<()> {
    let keyboard = InlineKeyboard::new([ActionRow::buttons([
        Button::callback("Confirm", "confirm").style(ButtonStyle::Success),
        Button::url("Documentation", "https://example.com/docs"),
    ])]);

    bot.send_message_with_options(
        vec![MessageSegment::text("Continue?")],
        SendMessageTarget::Private("user-id".to_owned()),
        MessageOptions::default().components(MessageComponents::InlineKeyboard(keyboard)),
    )
    .await?;
    Ok(())
}
```

The common component model includes callback, send-text, URL, Web App, login, inline-query, copy, game and payment buttons; text/user/role/channel selects; rich reply keyboards; modals; commands and menus. An adapter maps only semantics its platform can represent. `PlatformNativeData` is the lossless escape hatch for a platform field or component that has no portable meaning.

An interaction may carry an `InteractionResponseHandle`. The handle records
whether acknowledgement is required, its deadline, follow-up support and the
platform context needed to edit or delete the original response.
`InteractionEvent::respond` routes acknowledge/defer, initial messages,
message updates and simple callback responses through the appropriate
lifecycle method. Message-like reply-keyboard events intentionally have no
response handle, so they cannot accidentally be answered as callback queries.

### Event
`Event` is the object that the framework processes, representing the various events received by the bot. Event types include:

- **MessageEvent**: Message events
- **NoticeEvent**: Notification events
- **RequestEvent**: Request events
- **InteractionEvent**: Button, selection, command, and form interactions
- **LifecycleEvent**: v2 messages, reactions, pins, polls, checklists, threads,
  members, permissions, suggestions, Mini Apps, checkout, payment and
  subscription lifecycle events
- **MetaEvent**: Meta events
- **AnyEvent**: Generalized events

### Matcher
`Matcher` is an abstraction over `Bot` and `Event`, simplifying event handling and API calls. It provides convenient methods to extract key information from events (such as users, messages, groups) and easily call related APIs.

For v2 code, `try_get_conversation`, `try_send_outgoing_message` and
`try_reply_outgoing_message` preserve thread/topic addressing and rich message
options. `event_envelopes` exposes delivery metadata and raw payloads without
changing the public 0.1 `Matcher` layout. `EventEnvelope` includes a stable
event ID, platform, timestamps, conversation, delivery attempt, metadata and
the original platform JSON when an adapter supplies it.

### Native data and unsupported features

`PlatformNativeData` can be attached to content, components, targets,
conversations, replies, events and most option objects. An adapter must verify
the platform identifier and reject duplicate fields; it must never silently
interpret native data intended for another platform.

Use native data for a small platform-only field and `PlatformApiRequest` for a
complete platform-only workflow. Portable fields are intentionally strict:
when a platform cannot represent a requested semantic, the adapter returns a
structured error rather than dropping it. This makes the same business logic
safe to run against adapters with very different feature sets.

### Handler
`Handler` is the core component for event processing, divided into two types:

- **EventHandler**: Handles incoming `Event`s and is triggered only when an event occurs.
- **ActiveHandler**: Suitable for proactive processing scenarios, it can run continuously, execute scheduled tasks, or perform other background operations.

A `Handler` can include either an `EventHandler` or an `ActiveHandler`, or both.

### Filter
`Filter` is a global event filter used to process and intercept events before they reach the `Handler`. The `Filter` has a higher priority than the `Handler`.

### OxideBotManager
`OxideBotManager` is the manager of the framework, the entry point for starting and running the bot. Developers should call its `run_block` method at the end of the `main` function to launch the entire framework along with all registered `Bot`s, `Filter`s, and `Handler`s.

## Auxiliary Tools for Handler Writer

### Wait

Include a restricted `BroadcastSender` that can only use `subscribe` fn in your handler
```rust
use oxidebot::manager::BroadcastSender;

pub struct WaitHandler {
    pub broadcast_sender: BroadcastSender,
}
```

Then use `wait` in your `HandlerTrait` implementation.
You can find the provided `wait` methods in `utils::wait` or define your own.
```rust
use std::time::Duration;

use anyhow::Result;
use oxidebot::{manager::BroadcastSender, matcher::Matcher, wait_user_text_generic};

async fn wait_for_number(
    matcher: &Matcher,
    broadcast_sender: &BroadcastSender,
) -> Result<()> {
    let (number, matcher) = wait_user_text_generic::<u8>(
        matcher,
        broadcast_sender,
        Duration::from_secs(30),
        3,
        Some("Please send an unsigned 8-bit integer".to_string()),
    )
    .await?;
    # let _ = (number, matcher);
    Ok(())
}
```

## License
MIT OR Apache-2.0
