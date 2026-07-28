//! OxideBot's batteries-included facade.
//!
//! Most applications need `use oxidebot::prelude::*;`, one or more adapters,
//! a flat [`Module`], and ordinary async functions.

pub use oxidebot_core as core;
pub use oxidebot_core::*;
pub use oxidebot_runtime as runtime;
pub use oxidebot_runtime::*;

/// Derives a strongly typed command schema and parser.
pub use oxidebot_macros::{command, BotCommand, CommandArgs};

/// Typed values that ordinary handler functions may request.
pub mod extract {
    pub use oxidebot_core::{BotIdentity, EventId};
    pub use oxidebot_runtime::{
        Args, Bot, BranchArgs, ChatGroup, CommandRegistry, CommandResult, Context, Dialogue,
        EventContext, Extract, ExtractError, MaybeGroup, MessageContext, MessageId, Reply, Resolve,
        Segments, Sender, ShutdownSignal, State, Target, Text,
    };
}

/// Typed command schemas, values, parsing, and completion.
pub mod commands {
    pub use oxidebot_runtime::{
        command, value_pattern, when_branch, when_field_equals, ArgumentAction, ArgumentChoice,
        ArgumentSpec, BranchArgs, Command, CommandArgs, CommandBranch, CommandBranchTag,
        CommandCatalog, CommandFieldId, CommandId, CommandMatch, CommandNodeId, CommandOutput,
        CommandOverlay, CommandParseError, CommandRegistry, CommandRenderer, CommandResult,
        CommandSchema, CommandSource, CommandTree, CommandValue, CommandValueKind,
        CompletionConfig, CompletionInput, CompletionItem, CompletionKind, DefaultCommandRenderer,
        DynamicCompleter, FnValuePattern, FromCommandMatch, FromCommandValue, LocalizedText,
        Mention, ParsedArguments, RegexTextPattern, Resolve, ResolveCommandValue, Shortcut,
        ShortcutPattern, SourceSpan, UnitBranch, ValuePattern,
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
        ActionRow, Audios, BotId, BotIdentity, Button, ButtonAction, ButtonStyle, CallApiTrait,
        ChannelMentions, Checklist, Components, ContactCard, CustomEmoji, EventId, Files, Forwards,
        Images, InlineKeyboard, LocalizedMessage, LocationContent, Media, MediaGalleryItem,
        MediaType, MessageComponents, MessageTemplate, NativeSegments, PlatformId, Poll,
        PollOption, PollType, Polls, References, Replies, RichLayout, RichText, RichTextSegments,
        RoleMentions, SegmentSelector, SegmentTransform, Sticker, TemplateError, TemplateValue,
        TextSegments, TextSpan, TextStyle, TranslationCatalog, TranslationError, UserMentions,
        Videos,
    };
    pub use oxidebot_macros::{command, BotCommand, CommandArgs};
    pub use oxidebot_runtime::{
        command, command_admin_module, diagnostics_module, echo_module, language_module,
        resolve_and_host_file, resolve_message_media, shortcut_admin_module, value_pattern,
        when_branch, when_field_equals, Address, Args, Bot, BotDirectory, BotSelection, BranchArgs,
        ChatGroup, Command, CommandArgs, CommandBranch, CommandBranchTag, CommandMatch,
        CommandMiddleware, CommandOutput, CommandOutputMiddleware, CommandOverlay, CommandRegistry,
        CommandRenderer, CommandResult, CommandRewriter, CommandTree, CompletionConfig,
        CompletionInput, CompletionItem, CompletionKind, Context, DefaultCommandRenderer,
        DeliveryMiddleware, Dialogue, DynamicCompleter, EventContext, EventLocaleResolver, Extract,
        ExtractError, FnValuePattern, GuardDecision, GuardResult, HandlerError, HandlerResult,
        IntoOutcome, LocalMediaResolver, LocaleResolver, LocaleStorage, MediaFetcher, MediaHost,
        MediaResolver, MessageContext, MessageId, MessageNormalizer, Module, Outcome, OxideBot,
        PortableMediaResolver, Propagation, Receipt, RegexTextPattern, Reply, Resolve,
        ResolveCommandValue, ResolvedMedia, ResolvedMessageMedia, RewriteInput, Segments, Sender,
        SessionPolicy, Shortcut, ShortcutPattern, ShutdownSignal, State, StoredLocaleResolver,
        Target, TargetDirectory, Text, UnitBranch, ValuePattern,
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

/// Parses a structure-preserving message template once at the call site.
#[macro_export]
macro_rules! message_template {
    ($source:literal) => {{
        static TEMPLATE: ::std::sync::OnceLock<$crate::MessageTemplate> =
            ::std::sync::OnceLock::new();
        TEMPLATE
            .get_or_init(|| {
                $crate::MessageTemplate::parse($source)
                    .expect("static message template must be valid")
            })
            .clone()
    }};
}

/// Builds values for [`MessageTemplate::render`]. Use
/// `TemplateValue::mention(value)` for an explicit mention placeholder.
#[macro_export]
macro_rules! message_args {
    ($($name:ident => $value:expr),* $(,)?) => {{
        let mut values = ::std::collections::BTreeMap::new();
        $(values.insert(::std::sync::Arc::<str>::from(stringify!($name)), $crate::TemplateValue::from($value));)*
        values
    }};
}
