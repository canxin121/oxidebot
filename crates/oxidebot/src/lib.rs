//! OxideBot's batteries-included facade.
//!
//! Most applications need `use oxidebot::prelude::*;`, one or more adapters,
//! a flat [`Module`], and ordinary async functions.

pub use oxidebot_core as core;
pub use oxidebot_core::*;
pub use oxidebot_runtime as runtime;
pub use oxidebot_runtime::*;

/// Derives a strongly typed command schema and parser.
pub use oxidebot_macros::CommandArgs;

/// Typed values that ordinary handler functions may request.
pub mod extract {
    pub use oxidebot_core::{BotIdentity, EventId};
    pub use oxidebot_runtime::{
        Args, Bot, ChatGroup, CommandResult, Context, Dialogue, EventContext, Extract,
        ExtractError, MaybeGroup, MessageContext, MessageId, Reply, Segments, Sender,
        ShutdownSignal, State, Target, Text,
    };
}

/// Typed command schemas, values, parsing, and completion.
pub mod commands {
    pub use oxidebot_runtime::{
        command, ArgumentSpec, Command, CommandArgs, CommandCatalog, CommandParseError,
        CommandResult, CommandSchema, CommandValue, CompletionConfig, FromCommandValue, Mention,
        ParsedArguments,
    };
}

/// Flat Bot-module, admission, hook, and effect building blocks.
pub mod handler {
    pub use oxidebot_runtime::{
        After, Before, Guard, GuardDecision, GuardResult, IntoOutcome, Module, Outcome, Propagation,
    };
}

/// Imports intended for ordinary Bot application modules.
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
        command, Args, Bot, ChatGroup, Command, CommandArgs, CommandResult, CompletionConfig,
        Context, Dialogue, EventContext, Extract, ExtractError, GuardDecision, GuardResult,
        HandlerError, HandlerResult, IntoOutcome, MaybeGroup, Mention, MessageContext, MessageId,
        Module, Outcome, OxideBot, Propagation, Receipt, Reply, Segments, Sender, SessionPolicy,
        ShutdownSignal, State, Target, Text,
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
