use crate::{
    command::tokenize_segments,
    function::IntoHandler,
    handler::{ErasedHandler, HandlerCall, RouteScope, RouteSpec},
    hooks::{
        After, Before, Endpoint, Guard, GuardDecision, SharedAfter, SharedBefore, SharedGuard,
    },
    BuildError, Command, CommandCatalog, CommandFieldId, CommandFieldTag, CommandId, CommandMatch,
    CommandOutput, CommandParseError, CompletionConfig, CompletionInput, Context, Dialogue,
    DynamicCompleter, Extract, HandlerError, HandlerResult, Outcome, RewriteInput, Shortcut,
    SourceSpan,
};
use futures_util::future::BoxFuture;
use oxidebot_core::{
    event::{kernel::DispatchKind, tags, EventTag, EventType},
    interaction::InteractionKind,
    BotIdentity, Event, PlatformId,
};
use std::{collections::HashMap, marker::PhantomData, sync::Arc};

#[path = "module_command.rs"]
mod module_command;

use module_command::{
    command_completion_outcome, command_parse_outcome, explicit_completion_input,
    match_command_event, prepare_command, CommandEventError, CommandPreparation, HelpEndpoint,
    SuggestionEndpoint,
};

#[path = "module_model.rs"]
mod model;

#[path = "module_feature.rs"]
mod feature;

#[path = "module_builder.rs"]
mod builder;

#[path = "module_validation.rs"]
mod validation;

#[path = "module_handler.rs"]
mod handler_runtime;

pub use model::{Feature, FeatureExt, GeneratedFeature, IntoFeature, Module};

use handler_runtime::{EndpointKind, HandlerDefinition, ModuleHandler, Selector};
use model::CompleterBinding;
use validation::validate_command_conflicts;
