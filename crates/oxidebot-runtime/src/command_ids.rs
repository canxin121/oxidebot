//! Stable command-tree and argument identifiers.

/// A deterministic identifier for one command tree.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandId(pub u64);

/// A deterministic identifier for one node inside a command tree.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandNodeId(pub u32);

/// A schema-local identifier for one command argument.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommandFieldId(pub u32);

/// Compile-time marker generated for one field in a `CommandArgs` schema.
///
/// Field markers keep completion and async resolution next to the feature that
/// owns the command without exposing stringly typed field names in application
/// code. The stable runtime field ID is still resolved from the canonical
/// command tree, so help, parsing, completion, and platform commands share one
/// source of truth.
pub trait CommandFieldTag: Clone + Copy + Send + Sync + 'static {
    /// Generated field name as declared in the command schema.
    const NAME: &'static str;
}
