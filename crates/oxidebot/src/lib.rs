//! OxideBot's batteries-included facade.
//!
//! Most applications only need `use oxidebot::prelude::*;`, an adapter, a
//! [`Router`], and ordinary async functions.

pub use oxidebot_core as core;
pub use oxidebot_core::*;
pub use oxidebot_runtime as runtime;
pub use oxidebot_runtime::*;

/// Derives a strongly typed command schema and parser.
pub use oxidebot_macros::CommandArgs;

/// Common request extractors.
pub mod extract {
    pub use oxidebot_core::{BotIdentity, EventId};
    pub use oxidebot_runtime::{
        Bot, ChatGroup, CommandResult, Dialogue, EventContext, Extension, FromRef, FromRequest,
        MaybeGroup, MessageContext, MessageId, Parsed, Receipt, Reply, Segments, Sender,
        ShutdownSignal, State, Target, Text,
    };
}

/// Router and middleware building blocks.
pub mod routing {
    pub use oxidebot_runtime::{
        command, event, from_fn, interaction, message, native, Command, CommandCatalog,
        CompletionConfig, Middleware, Next, Plugin, Request, Response, Router,
    };
}

/// Imports intended for bot application modules.
pub mod prelude {
    pub use crate::message;
    pub use oxidebot_core::{
        event::{self, tags, Event},
        source::{
            group::Group,
            message::{File, IntoMessageSegment, Message, MessageSegment},
            user::User,
        },
        BotId, BotIdentity, CallApiTrait, EventId, PlatformId,
    };
    pub use oxidebot_macros::CommandArgs;
    pub use oxidebot_runtime::{
        command, event as on_event, from_fn, interaction, message as on_message, native,
        ArgumentSpec, Bot, ChatGroup, Command, CommandArgs as CommandArgsTrait, CommandCatalog,
        CommandParseError, CommandResult, CommandSchema, CommandValue, CompletionConfig, Dialogue,
        EventContext, Extension, Extensions, FromCommandValue, FromRef, FromRequest, HandlerError,
        HandlerResult, IntoResponse, MaybeGroup, Mention, MessageContext, MessageId, Middleware,
        Next, OxideBot, Parsed, ParsedArguments, Plugin, Receipt, Reply, Request, Response, Router,
        Segments, Sender, SessionPolicy, ShutdownSignal, State, Target, Text,
    };
}

/// Builds the canonical 0.1.8 message type from text and message segments.
#[macro_export]
macro_rules! message {
    ($($part:expr),* $(,)?) => {{
        let mut message = $crate::core::source::message::Message::default();
        $(message.push($part);)*
        message
    }};
}
