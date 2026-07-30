//! Root command-tree construction, validation, matching, and publication.

use super::*;

#[path = "command_definition_command_core.rs"]
mod core;

#[path = "command_definition_command_validation.rs"]
mod validation;

#[path = "command_definition_command_match.rs"]
mod matching;

#[path = "command_definition_command_publication.rs"]
mod publication;

pub use core::{command, Command};
