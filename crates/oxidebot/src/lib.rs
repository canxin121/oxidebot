//! OxideBot's batteries-included facade.
//!
//! Most applications need `use oxidebot::prelude::*;`, one or more adapters,
//! a flat [`Module`], and ordinary async functions.

pub use oxidebot_core as core;
pub use oxidebot_core::*;
pub use oxidebot_runtime as runtime;
pub use oxidebot_runtime::*;

/// Derives a strongly typed command schema and parser.
pub use oxidebot_macros::{BotCommand, CommandArgs};

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
        command, ArgumentAction, ArgumentChoice, ArgumentSpec, Command, CommandArgs, CommandBranch,
        CommandCatalog, CommandFieldId, CommandId, CommandMatch, CommandNodeId, CommandOutput,
        CommandParseError, CommandRenderer, CommandResult, CommandSchema, CommandSource,
        CommandTree, CommandValue, CommandValueKind, CompletionConfig, CompletionItem,
        CompletionKind, DefaultCommandRenderer, FromCommandMatch, FromCommandValue, LocalizedText,
        Mention, ParsedArguments, SourceSpan,
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
            message::{
                DegradationKind, DeliveryDegradation, DeliveryPlan, DeliveryPlanningError,
                DeliveryReport, FallbackPolicy, File, IntoMessageSegment, Message, MessageOptions,
                MessageSegment, SegmentKind,
            },
            user::User,
        },
        ActionRow, BotId, BotIdentity, Button, ButtonAction, ButtonStyle, CallApiTrait, Checklist,
        ContactCard, CustomEmoji, EventId, InlineKeyboard, LocationContent, Media,
        MediaGalleryItem, MediaType, MessageComponents, PlatformId, Poll, PollOption, PollType,
        RichLayout, RichText, Sticker, TextSpan, TextStyle,
    };
    pub use oxidebot_macros::{BotCommand, CommandArgs};
    pub use oxidebot_runtime::{
        command, Args, Bot, ChatGroup, Command, CommandArgs, CommandBranch, CommandMatch,
        CommandResult, CommandTree, CompletionConfig, CompletionItem, CompletionKind, Context,
        Dialogue, EventContext, Extract, ExtractError, GuardDecision, GuardResult, HandlerError,
        HandlerResult, IntoOutcome, MaybeGroup, Mention, MessageContext, MessageId, Module,
        Outcome, OxideBot, Propagation, Receipt, Reply, Segments, Sender, SessionPolicy,
        ShutdownSignal, State, Target, Text,
    };
}

/// Builds the unified cross-platform message IR from text and message segments.
#[macro_export]
macro_rules! message {
    ($($part:expr),* $(,)?) => {{
        let mut message = $crate::core::source::message::Message::default();
        $(message.push($part);)*
        message
    }};
}
