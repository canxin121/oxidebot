//! Trait contracts implemented by typed command arguments and command trees.

use super::*;

/// Converts the shared structured match into one handler argument.
pub trait FromCommandMatch: Sized + Send + 'static {
    /// Converts one structured command match into this handler argument type.
    fn from_match(result: &CommandMatch) -> Result<Self, CommandParseError>;
}

/// Implemented by strongly typed flat command argument structs.
pub trait CommandArgs: Sized + Send + 'static {
    /// Returns the static schema for this flat argument struct.
    fn schema() -> CommandSchema;
    /// Builds this struct from parsed values.
    fn from_arguments(arguments: &ParsedArguments) -> Result<Self, CommandParseError>;

    /// Parses a command match using this type's static schema.
    fn parse(result: &CommandMatch) -> Result<Self, CommandParseError> {
        let arguments = result.parse_with(&Self::schema())?;
        Self::from_arguments(&arguments)
    }

    /// Creates a command named `name` bound to this type's schema.
    #[must_use]
    fn command(name: impl Into<Arc<str>>) -> Command {
        Command::new(name).schema(Self::schema())
    }

    /// Defines and binds a flat argument command in one expression.
    #[must_use]
    fn feature<S, H, T>(name: impl Into<Arc<str>>, handler: H) -> crate::Feature<S>
    where
        S: Send + Sync + 'static,
        H: crate::IntoHandler<T, S>,
    {
        crate::Feature::command(Self::command(name), handler)
    }
}

impl<T> FromCommandMatch for T
where
    T: CommandArgs,
{
    fn from_match(result: &CommandMatch) -> Result<Self, CommandParseError> {
        let arguments = if let Some(arguments) = result.arguments() {
            arguments.clone()
        } else {
            result.parse_active()?
        };
        T::from_arguments(&arguments)
    }
}

/// Implemented by enums derived with `#[derive(BotCommand)]`.
///
/// The name deliberately describes the command grammar rather than colliding
/// with the platform menu `BotCommand` model from `oxidebot-core`.
pub trait CommandTree: FromCommandMatch {
    /// Returns the complete typed command-tree grammar.
    fn command() -> Command;

    /// Defines and binds the complete typed command tree in one expression.
    #[must_use]
    fn feature<S, H, T>(handler: H) -> crate::Feature<S>
    where
        S: Send + Sync + 'static,
        H: crate::IntoHandler<T, S>,
    {
        crate::Feature::command(Self::command(), handler)
    }
}
