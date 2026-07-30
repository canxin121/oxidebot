//! Focused command-parser fixtures that do not start adapters or executors.

use oxidebot_core::Message;
use oxidebot_runtime::{Command, CommandArgs, CommandParseError, CommandTree, FromCommandMatch};
use std::{marker::PhantomData, sync::Arc};

/// Parser harness for one immutable command definition.
pub struct CommandTest<T> {
    command: Command,
    _value: PhantomData<fn() -> T>,
}

impl<T> CommandTest<T>
where
    T: FromCommandMatch,
{
    /// Builds a parser harness for one immutable command definition.
    #[must_use]
    pub fn new(command: Command) -> Self {
        Self {
            command,
            _value: PhantomData,
        }
    }

    /// Parses input with the same command IR used by the runtime.
    pub fn parse(&self, input: impl Into<String>) -> Result<T, CommandParseError> {
        let matched = self.command.parse_message(&Message::text(input.into()))?;
        T::from_match(&matched)
    }

    /// Returns the command definition exercised by this harness.
    #[must_use]
    pub fn command(&self) -> &Command {
        &self.command
    }
}

/// Builds a parser harness for one flat [`CommandArgs`] type.
#[must_use]
pub fn command_test<T>(name: impl Into<Arc<str>>) -> CommandTest<T>
where
    T: CommandArgs,
{
    CommandTest::new(T::command(name))
}

/// Builds a parser harness for one derived [`CommandTree`].
#[must_use]
pub fn command_tree_test<T>() -> CommandTest<T>
where
    T: CommandTree,
{
    CommandTest::new(T::command())
}
