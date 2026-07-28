//! OxideBot's batteries-included facade.
//!
//! Most applications need `use oxidebot::prelude::*;`, one or more adapters,
//! a flat [`Module`], and ordinary async functions.

pub use oxidebot_core as core;
pub use oxidebot_core::*;
pub use oxidebot_runtime as runtime;
pub use oxidebot_runtime::*;

/// Derives a strongly typed command schema and parser.
pub use oxidebot_macros::{
    branch, command, completer, BotCommand, BotState, CommandArgs, DialogueForm,
};

/// Typed values that ordinary handler functions may request.
pub mod extract {
    pub use oxidebot_core::{BotIdentity, EventId};
    pub use oxidebot_runtime::{
        Args, Bot, BranchArgs, ChatGroup, CommandRegistry, CommandResult, ConfirmationWords,
        Context, Dialogue, DialogueForm, DialogueFormFuture, DialogueQuestion, EventContext,
        Extract, ExtractError, FromState, I18n, I18nMessage, MaybeGroup, MessageContext, MessageId,
        Messenger, OptionExt, Reply, Resolve, ResultExt, Segments, Sender, ShutdownSignal, State,
        Target, Text,
    };
}

/// Typed command schemas, values, parsing, completion, shortcuts, and branch
/// dispatch.
pub mod commands {
    pub use oxidebot_runtime::{
        command, value_pattern, when_branch, when_field_equals, ArgumentAction, ArgumentChoice,
        ArgumentSpec, BranchArgs, CatalogCommandRenderer, Command, CommandArgs, CommandBranch,
        CommandBranchTag, CommandCatalog, CommandFieldId, CommandFieldTag, CommandId, CommandMatch,
        CommandNodeId, CommandOutput, CommandOverlay, CommandParseError, CommandRegistry,
        CommandRenderer, CommandResult, CommandSchema, CommandSource, CommandTree, CommandValue,
        CommandValueKind, CompletionConfig, CompletionInput, CompletionItem, CompletionKind,
        DefaultCommandRenderer, DynamicCompleter, FnValuePattern, FromCommandMatch,
        FromCommandValue, LocalizedText, Mention, ParsedArguments, RegexTextPattern, Resolve,
        ResolveCommandValue, Shortcut, ShortcutPattern, SourceSpan, UnitBranch, ValuePattern,
    };

    pub mod prelude {
        pub use super::{
            command, ArgumentAction, ArgumentChoice, ArgumentSpec, BranchArgs, Command,
            CommandArgs, CommandBranchTag, CommandFieldTag, CommandMatch, CommandOutput,
            CommandRegistry, CommandTree, CompletionConfig, CompletionInput, CompletionItem,
            CompletionKind, FromCommandValue, LocalizedText, Mention, Resolve, ResolveCommandValue,
            Shortcut, ValuePattern,
        };
        pub use oxidebot_macros::{branch, command, completer, BotCommand, CommandArgs};
    }
}

/// Unified cross-platform message construction, inspection, templates, and
/// localization.
pub mod message {
    pub use oxidebot_core::{
        source::message::{
            File, IntoMessageSegment, Message, MessageOptions, MessageSegment, SegmentKind,
        },
        ActionRow, Audios, Button, ButtonAction, ButtonStyle, ChannelMentions, Checklist,
        Components, ContactCard, CustomEmoji, Files, Forwards, Images, InlineKeyboard,
        LocalizedMessage, LocationContent, Media, MediaGalleryItem, MediaType, MessageComponents,
        MessageTemplate, NativeSegments, Poll, PollOption, PollType, Polls, References, Replies,
        RichLayout, RichText, RichTextSegments, RoleMentions, SegmentSelector, SegmentTransform,
        Sticker, TemplateError, TemplateValue, TextSegments, TextSpan, TextStyle,
        TranslationCatalog, TranslationError, UserMentions, Videos,
    };

    pub mod prelude {
        pub use super::{
            ActionRow, Button, File, InlineKeyboard, LocalizedMessage, Media, Message,
            MessageComponents, MessageOptions, MessageSegment, MessageTemplate, SegmentKind,
            TemplateValue, TranslationCatalog,
        };
        pub use crate::{button, message_args, message_template, row};
    }
}

/// Capability-aware delivery, proactive addressing, receipts, and media
/// resolution.
pub mod delivery {
    pub use oxidebot_core::source::message::{
        DegradationKind, DeliveryDegradation, DeliveryPlan, DeliveryPlanningError, DeliveryReport,
        FallbackPolicy,
    };
    pub use oxidebot_runtime::{
        resolve_and_host_file, resolve_message_media, Address, BotDirectory, BotSelection,
        DeliveryMiddleware, LocalMediaResolver, MediaFetcher, MediaHost, MediaResolver, Messenger,
        PortableMediaResolver, Receipt, Reply, ResolvedMedia, ResolvedMessageMedia,
        TargetDirectory,
    };

    pub mod prelude {
        pub use super::{
            Address, BotDirectory, BotSelection, DeliveryReport, FallbackPolicy, Messenger,
            Receipt, Reply, TargetDirectory,
        };
    }
}

/// Focused imports for platform-adapter and transport authors.
pub mod adapter {
    pub use oxidebot_runtime::{
        Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices, FrameIndex,
        InboundFrame, MessageFrame, MessageFrameBuilder, Submission,
    };

    pub mod prelude {
        pub use super::{
            Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices,
            MessageFrame, MessageFrameBuilder,
        };
        pub use oxidebot_core::{BotId, EventId, Message, PlatformId};
    }
}

/// Flat Bot-module, admission, hook, and effect building blocks.
pub mod handler {
    pub use oxidebot_runtime::{
        After, Before, Feature, FeatureExt, GeneratedFeature, Guard, GuardDecision, GuardResult,
        IntoFeature, IntoOutcome, Module, Outcome, Propagation,
    };
}

/// Optional batteries-included modules for help-adjacent application tools.
/// They are ordinary flat [`Module`](crate::Module) values and do not create a
/// second plugin runtime.
pub mod standard {
    pub use oxidebot_runtime::{
        command_admin_module, diagnostics_module, echo_module, language_module,
        shortcut_admin_module, AdminTools, CommandAdminArguments, EchoArguments, LanguageArguments,
        LocaleStorage, ShortcutArguments, StoredLocaleResolver,
    };
}

/// Less frequently needed runtime, adapter, middleware, publication, and
/// resource-control APIs. Keeping these out of the ordinary prelude makes IDE
/// completion reflect the common authoring path.
pub mod advanced {
    pub use oxidebot_runtime::{
        Adapter, AdapterContext, AdapterError, AdapterMode, AdminTools, BotDescriptor, BotServices,
        CatalogCommandRenderer, CommandMiddleware, CommandOutputMiddleware, CommandOverlay,
        CommandRewriter, DeliveryMiddleware, FrameIndex, InboundFrame, MessageFrame,
        MessageFrameBuilder, MessageNormalizer, MetricsHandle, RuntimeConfig, RuntimeProfile,
        Service, ServiceContext,
    };
}

/// Imports intended for ordinary Bot application modules.
///
/// Specialized command, rich-message, delivery, and adapter APIs are available
/// from `oxidebot::commands::prelude`, `oxidebot::message::prelude`,
/// `oxidebot::delivery::prelude`, `oxidebot::adapter::prelude`, and
/// `oxidebot::advanced`.
pub mod prelude {
    pub use crate::{button, message, message_args, message_template, row};
    pub use oxidebot_core::{
        event::{self, tags, Event},
        source::{
            group::Group,
            message::{File, Message, MessageOptions, MessageSegment, SegmentKind},
            user::User,
        },
        BotId, BotIdentity, CallApiTrait, EventId, PlatformId,
    };
    pub use oxidebot_macros::{
        branch, command, completer, BotCommand, BotState, CommandArgs, DialogueForm,
    };
    pub use oxidebot_runtime::{
        command, Args, Bot, BranchArgs, ChatGroup, Command, CommandArgs, CommandBranchTag,
        CommandTree, CompletionConfig, ConfirmationWords, Context, Dialogue, DialogueForm,
        DialogueQuestion, EventContext, Extract, ExtractError, Feature, FeatureExt, FromState,
        GuardDecision, GuardResult, HandlerError, HandlerResult, I18n, MaybeGroup, MessageContext,
        MessageId, Messenger, Module, OptionExt, Outcome, OxideBot, Propagation, Receipt, Reply,
        ResultExt, Segments, Sender, SessionPolicy, ShutdownSignal, State, Target, Text,
    };
}

/// Compatibility import that intentionally exposes the complete public surface.
/// New applications should prefer the focused preludes above.
pub mod all {
    pub use crate::{button, message, message_args, message_template, row};
    pub use oxidebot_core::*;
    pub use oxidebot_macros::{
        branch, command, completer, BotCommand, BotState, CommandArgs, DialogueForm,
    };
    pub use oxidebot_runtime::*;
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

/// Builds one interactive button with a compact domain-oriented syntax.
#[macro_export]
macro_rules! button {
    ($label:expr => url($value:expr)) => {
        $crate::Button::url($label, $value)
    };
    ($label:expr => action($value:expr)) => {
        $crate::Button::callback($label, $value)
    };
    ($label:expr => text($value:expr)) => {
        $crate::Button::send_text($label, $value)
    };
}

/// Builds one action row from buttons.
#[macro_export]
macro_rules! row {
    ($($button:expr),* $(,)?) => {
        $crate::ActionRow::buttons([$($button),*])
    };
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
