//! Strongly typed, asynchronous command-value resolution.
//!
//! Synchronous command parsing remains allocation-light; this module provides
//! the explicit cold-path extractor and reusable value patterns for values
//! that need application state or asynchronous I/O.

use crate::{
    CommandFieldId, CommandMatch, CommandParseError, Context, Extract, ExtractError, HandlerError,
    HandlerResult,
};
use async_trait::async_trait;
use regex::Regex;
use std::{marker::PhantomData, sync::Arc};

/// Async command-value resolver. It remains an explicit cold-path operation;
/// ordinary `Args<T>` extraction stays synchronous and allocation-free.
#[async_trait]
pub trait ResolveCommandValue<S>: Sized + Send + 'static
where
    S: Send + Sync + 'static,
{
    /// Resolves the active command `field` from the current parsed match.
    async fn resolve(
        context: &Context<S>,
        field: CommandFieldId,
        command: &CommandMatch,
    ) -> HandlerResult<Self>;
}

/// Extractor-backed helper for asynchronous command-value resolution.
#[derive(Clone)]
pub struct Resolve<T, S>
where
    S: Send + Sync + 'static,
{
    context: Context<S>,
    _value: PhantomData<fn() -> T>,
}

impl<T, S> Resolve<T, S>
where
    T: ResolveCommandValue<S>,
    S: Send + Sync + 'static,
{
    /// Resolves a command field by its stable runtime ID.
    pub async fn field(&self, field: CommandFieldId) -> HandlerResult<T> {
        let command = self
            .context
            .command()
            .ok_or_else(|| HandlerError::Parse("resolver requires a command match".into()))?;
        T::resolve(&self.context, field, command).await
    }

    /// Resolves a field through its macro-generated static marker.
    pub async fn get<F>(&self, _field: F) -> HandlerResult<T>
    where
        F: crate::CommandFieldTag,
    {
        let command = self
            .context
            .command()
            .ok_or_else(|| HandlerError::Parse("resolver requires a command match".into()))?;
        let branch = command
            .branch_names()
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>();
        let field = command
            .command()
            .field_id(&branch, F::NAME)
            .ok_or_else(|| {
                HandlerError::Parse(format!(
                    "command field `{}` is not active on branch `{}`",
                    F::NAME,
                    branch.join(" "),
                ))
            })?;
        T::resolve(&self.context, field, command).await
    }
}

impl<T, S> Extract<S> for Resolve<T, S>
where
    T: ResolveCommandValue<S>,
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(Self {
            context: context.clone(),
            _value: PhantomData,
        })
    }
}

/// A reusable validation and conversion pattern for command values.
pub trait ValuePattern<T>: Send + Sync + 'static {
    /// Parses one portable command value into `T`.
    fn parse(&self, value: &crate::CommandValue) -> Result<T, CommandParseError>;
    /// Returns human-readable validation guidance for this pattern.
    fn describe(&self) -> Arc<str>;
}

/// A [`ValuePattern`] backed by a parsing closure.
pub struct FnValuePattern<T, F> {
    description: Arc<str>,
    parser: F,
    _value: PhantomData<fn() -> T>,
}

/// Creates a closure-backed value pattern with user-facing `description`.
#[must_use]
pub fn value_pattern<T, F>(description: impl Into<Arc<str>>, parser: F) -> FnValuePattern<T, F>
where
    F: Fn(&crate::CommandValue) -> Result<T, CommandParseError> + Send + Sync + 'static,
{
    FnValuePattern {
        description: description.into(),
        parser,
        _value: PhantomData,
    }
}

impl<T, F> ValuePattern<T> for FnValuePattern<T, F>
where
    T: Send + Sync + 'static,
    F: Fn(&crate::CommandValue) -> Result<T, CommandParseError> + Send + Sync + 'static,
{
    fn parse(&self, value: &crate::CommandValue) -> Result<T, CommandParseError> {
        (self.parser)(value)
    }

    fn describe(&self) -> Arc<str> {
        Arc::clone(&self.description)
    }
}

/// Text value pattern that accepts values matching a compiled regular expression.
#[derive(Clone, Debug)]
pub struct RegexTextPattern {
    regex: Regex,
    description: Arc<str>,
}

impl RegexTextPattern {
    /// Compiles `pattern` and stores `description` for validation failures.
    pub fn new(pattern: &str, description: impl Into<Arc<str>>) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: Regex::new(pattern)?,
            description: description.into(),
        })
    }
}

impl ValuePattern<String> for RegexTextPattern {
    fn parse(&self, value: &crate::CommandValue) -> Result<String, CommandParseError> {
        let Some(text) = value.as_text() else {
            return Err(CommandParseError::UnexpectedValue {
                expected: "text",
                actual: "non-text command value",
            });
        };
        if self.regex.is_match(text) {
            Ok(text.to_owned())
        } else {
            Err(CommandParseError::InvalidValue {
                value: text.to_owned(),
                expected: "matching text",
                reason: self.description.to_string(),
            })
        }
    }

    fn describe(&self) -> Arc<str> {
        Arc::clone(&self.description)
    }
}
