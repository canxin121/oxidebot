use crate::Shortcut;
use oxidebot_core::{
    application::{
        CommandChoice, CommandContext, CommandDefinition, CommandInvocation, CommandKind,
        CommandOption, CommandOptionType, Localized, Suggestion,
    },
    conversation::{ConversationKind, ConversationRef},
    source::{
        message::{File, Message, MessageSegment},
        user::User,
    },
    FormValue,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;

#[path = "command_parse.rs"]
mod command_parse;

#[path = "command_render.rs"]
mod command_render;

#[path = "command_catalog.rs"]
mod command_catalog;

#[path = "command_definition.rs"]
mod command_definition;

#[path = "command_completion.rs"]
mod command_completion;

#[path = "command_schema.rs"]
mod command_schema;

pub use command_catalog::CommandCatalog;
use command_completion::{branch_children, cursor_prefix, suggest_for_command};
pub use command_completion::{CompletionConfig, CompletionItem, CompletionKind, SourceSpan};
pub use command_definition::{
    command, ArgumentAction, ArgumentChoice, ArgumentGroup, Command, CommandBranch, CommandEnum,
    CommandValueKind, LocalizedText,
};
use command_definition::{normalize_native_name, same_word, stable_hash};
use command_parse::{
    branch_choice_list, parse_arguments, parse_native_arguments, unknown_subcommand,
    validate_argument_value,
};
pub use command_parse::{tokenize_segments, tokenize_text};
pub use command_render::{
    CatalogCommandRenderer, CommandOutput, CommandRenderer, DefaultCommandRenderer,
};
pub use command_schema::{ArgumentSpec, CommandSchema};

#[path = "command_ids.rs"]
mod ids;

#[path = "command_input.rs"]
mod input;

#[path = "command_arguments.rs"]
mod arguments;

#[path = "command_match.rs"]
mod matched;

#[path = "command_traits.rs"]
mod traits;

#[path = "command_error.rs"]
mod error;

pub use arguments::ParsedArguments;
pub use error::{CommandParseError, CompletionSuggestion};
pub use ids::{CommandFieldId, CommandFieldTag, CommandId, CommandNodeId};
pub use input::{CommandValue, FromCommandValue, Mention};
pub use matched::{CommandMatch, CommandSource};
pub use traits::{CommandArgs, CommandTree, FromCommandMatch};

use input::{form_value_label, value_kind};
