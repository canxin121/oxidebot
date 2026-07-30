//! Immutable command-tree definitions and builder validation.
//!
//! This module owns localized schema metadata, branches, command construction,
//! native publication definitions, and command-tree invariants. Parsing,
//! rendering, catalog lookup, and asynchronous value resolution stay separate.

use super::*;

#[path = "command_definition_types.rs"]
mod types;

#[path = "command_definition_branch.rs"]
mod branch;

#[path = "command_definition_command.rs"]
mod command;

pub use branch::CommandBranch;
pub use command::{command, Command};
pub use types::{
    ArgumentAction, ArgumentChoice, ArgumentGroup, CommandEnum, CommandValueKind, LocalizedText,
};

pub(super) fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn validate_global_schema_for_branch(
    global: &CommandSchema,
    branch: &CommandBranch,
    root_name: &str,
) -> Result<(), String> {
    if let Some(schema) = &branch.schema {
        global.merged(schema).validate().map_err(|error| {
            format!(
                "command `{root_name}` global arguments conflict with branch `{}`: {error}",
                branch.name
            )
        })?;
    }
    for child in &branch.children {
        validate_global_schema_for_branch(global, child, root_name)?;
    }
    Ok(())
}

fn collect_schema_field_ids(
    schema: &CommandSchema,
    path: &str,
    ids: &mut HashMap<CommandFieldId, String>,
) -> Result<(), String> {
    for argument in schema.arguments() {
        let field_path = format!("{path}.{}", argument.name());
        if let Some(previous) = ids.insert(argument.id(), field_path.clone()) {
            return Err(format!(
                "command grammar field ID {:?} collides between `{previous}` and `{field_path}`",
                argument.id(),
            ));
        }
    }
    Ok(())
}

fn collect_branch_ids(
    branch: &CommandBranch,
    path: &str,
    node_ids: &mut HashMap<CommandNodeId, String>,
    field_ids: &mut HashMap<CommandFieldId, String>,
) -> Result<(), String> {
    if let Some(previous) = node_ids.insert(branch.id(), path.to_owned()) {
        return Err(format!(
            "command grammar node ID {:?} collides between `{previous}` and `{path}`",
            branch.id(),
        ));
    }
    if let Some(schema) = branch.schema_ref() {
        collect_schema_field_ids(schema, path, field_ids)?;
    }
    for child in branch.children() {
        collect_branch_ids(
            child,
            &format!("{path} {}", child.name()),
            node_ids,
            field_ids,
        )?;
    }
    Ok(())
}

pub(super) fn normalize_native_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .replace('_', "-")
}
pub(super) fn same_word(actual: &str, expected: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        actual == expected
    } else {
        actual.eq_ignore_ascii_case(expected)
    }
}

/// Returns the number of tokens occupied by one global named option while a
/// command tree is still looking for its next subcommand. The caller retains
/// those tokens for ordinary argument parsing after the branch path is known.
fn global_option_width(
    schema: &CommandSchema,
    tokens: &[CommandValue],
    index: usize,
) -> Option<usize> {
    let text = tokens.get(index)?.as_text()?;
    if let Some(option) = text.strip_prefix("--").filter(|option| !option.is_empty()) {
        let (name, attached) = option
            .split_once('=')
            .map_or((option, None), |(name, value)| (name, Some(value)));
        let spec = schema
            .arguments()
            .iter()
            .find(|argument| argument.long_name() == Some(name))?;
        return Some(if spec.is_flag() || attached.is_some() {
            1
        } else {
            2
        });
    }
    let shorts = text.strip_prefix('-').filter(|shorts| !shorts.is_empty())?;
    if text.parse::<f64>().is_ok() {
        return None;
    }
    let characters = shorts.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while let Some(short) = characters.get(index) {
        let spec = schema
            .arguments()
            .iter()
            .find(|argument| argument.short_name() == Some(*short))?;
        if !spec.is_flag() {
            return Some(if index + 1 == characters.len() { 2 } else { 1 });
        }
        index += 1;
    }
    Some(1)
}
