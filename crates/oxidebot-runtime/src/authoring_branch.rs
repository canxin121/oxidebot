//! Strongly typed helpers for generated command-tree branches.

use super::*;

/// Compile-time metadata for one generated command-tree branch.
pub trait CommandBranchTag: Send + Sync + Sized + 'static {
    /// Generated command tree that owns this branch.
    type Command: crate::CommandTree;
    /// Generated arguments parsed for this branch.
    type Arguments: FromCommandMatch;
    /// Case-insensitive branch path beneath the command root.
    const PATH: &'static [&'static str];
    /// Whether this branch handler also matches descendant paths.
    const MATCH_DESCENDANTS: bool = false;
    /// Number of path components removed before parsing [`Self::Arguments`].
    const STRIP_PREFIX: usize = 0;

    /// Binds this generated branch marker to its handler as one feature.
    #[must_use]
    fn handle<S, H, T>(self, handler: H) -> crate::Feature<S>
    where
        S: Send + Sync + 'static,
        H: crate::IntoHandler<T, S>,
    {
        crate::Feature::command_branch(self, handler)
    }
}

/// Empty argument type for a command branch without fields.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnitBranch;

impl FromCommandMatch for UnitBranch {
    fn from_match(_result: &CommandMatch) -> Result<Self, CommandParseError> {
        Ok(Self)
    }
}

/// Extracted, strongly typed arguments for a generated command branch.
#[derive(Clone, Debug)]
pub struct BranchArgs<B>(pub B::Arguments)
where
    B: CommandBranchTag;

impl<B> Deref for BranchArgs<B>
where
    B: CommandBranchTag,
{
    type Target = B::Arguments;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S, B> Extract<S> for BranchArgs<B>
where
    S: Send + Sync + 'static,
    B: CommandBranchTag,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        let command = context
            .command()
            .ok_or_else(|| ExtractError::new("BranchArgs requires a command match"))?;
        let matches = if B::MATCH_DESCENDANTS {
            command.branch_names().len() >= B::PATH.len()
                && command
                    .branch_names()
                    .iter()
                    .take(B::PATH.len())
                    .zip(B::PATH)
                    .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
        } else {
            command.is_branch(B::PATH)
        };
        if !matches {
            return Err(ExtractError::new(
                "the selected command branch does not match this handler",
            ));
        }
        let command = command.descend(B::STRIP_PREFIX);
        B::Arguments::from_match(&command)
            .map(Self)
            .map_err(|error| ExtractError::new(error.localized_message(command.locale())))
    }
}

/// Creates a guard that admits only one selected command-tree path.
#[must_use]
pub fn when_branch<S>(path: impl IntoIterator<Item = impl Into<Arc<str>>>) -> impl Guard<S>
where
    S: Send + Sync + 'static,
{
    let path: Arc<[Arc<str>]> = path.into_iter().map(Into::into).collect::<Vec<_>>().into();
    move |context: Context<S>| {
        let path = Arc::clone(&path);
        async move {
            let Some(command) = context.command() else {
                return GuardDecision::skip();
            };
            let expected = path.iter().map(AsRef::as_ref).collect::<Vec<_>>();
            if command.is_branch(&expected) {
                GuardDecision::allow()
            } else {
                GuardDecision::skip()
            }
        }
    }
}

/// Creates a statically typed command-result condition without introducing
/// stringly typed handler state.
#[must_use]
pub fn when_field_equals<S, T>(field: CommandFieldId, expected: T) -> impl Guard<S>
where
    S: Send + Sync + 'static,
    T: FromCommandValue + Clone + Eq + Send + Sync + 'static,
{
    move |context: Context<S>| {
        let expected = expected.clone();
        async move {
            let Some(command) = context.command() else {
                return Ok::<GuardDecision, HandlerError>(GuardDecision::skip());
            };
            let arguments = if let Some(arguments) = command.arguments() {
                arguments.clone()
            } else {
                command
                    .parse_active()
                    .map_err(|error| HandlerError::Parse(error.to_string()))?
            };
            let value = arguments
                .optional_id::<T>(field)
                .map_err(|error| HandlerError::Parse(error.to_string()))?;
            Ok(if value.as_ref() == Some(&expected) {
                GuardDecision::allow()
            } else {
                GuardDecision::skip()
            })
        }
    }
}
