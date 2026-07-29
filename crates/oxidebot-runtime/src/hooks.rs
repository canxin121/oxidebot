use crate::{Context, HandlerError, HandlerResult, Outcome};
use futures_util::future::BoxFuture;
use oxidebot_core::source::message::Message;
use std::{future::Future, sync::Arc};

#[doc(hidden)]
pub trait Endpoint<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>>;
}

/// Decision returned by a Bot guard.
#[derive(Clone, Debug)]
pub enum GuardDecision {
    /// Run the matched handler.
    Allow,
    /// Do not run the matched handler and continue looking for another match.
    Skip,
    /// Do not run the matched handler and stop dispatch with these effects.
    Deny(Outcome),
}

impl GuardDecision {
    /// Creates a decision that permits the matched handler to run.
    #[must_use]
    pub const fn allow() -> Self {
        Self::Allow
    }

    /// Creates a decision that skips this handler and continues matching.
    #[must_use]
    pub const fn skip() -> Self {
        Self::Skip
    }

    /// Creates a stopping denial that replies with `message`.
    #[must_use]
    pub fn deny(message: impl Into<Message>) -> Self {
        Self::Deny(Outcome::stop().reply(message))
    }

    /// Creates a stopping denial with no reply effects.
    #[must_use]
    pub const fn deny_silently() -> Self {
        Self::Deny(Outcome::stop())
    }
}

/// Result returned by a guard check.
pub type GuardResult = HandlerResult<GuardDecision>;

#[doc(hidden)]
pub trait GuardOutput {
    fn into_guard_result(self) -> GuardResult;
}

impl GuardOutput for GuardDecision {
    fn into_guard_result(self) -> GuardResult {
        Ok(self)
    }
}

impl<E> GuardOutput for Result<GuardDecision, E>
where
    E: Into<HandlerError>,
{
    fn into_guard_result(self) -> GuardResult {
        self.map_err(Into::into)
    }
}

/// Async admission rule for one feature module.
///
/// Guards model Bot concerns such as permissions, rate limits, private/group
/// restrictions, and feature flags. They run before interactive completion and
/// handler argument extraction. Infallible guards return [`GuardDecision`]
/// directly; fallible guards return [`GuardResult`].
pub trait Guard<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Evaluates whether a matched handler may run for `context`.
    fn check(&self, context: Context<S>) -> BoxFuture<'static, GuardResult>;
}

impl<S, F, Fut, Output> Guard<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Output> + Send + 'static,
    Output: GuardOutput + 'static,
{
    fn check(&self, context: Context<S>) -> BoxFuture<'static, GuardResult> {
        let future = (self)(context);
        Box::pin(async move { future.await.into_guard_result() })
    }
}

#[doc(hidden)]
pub trait BeforeOutput {
    fn into_before_result(self) -> HandlerResult<()>;
}

impl BeforeOutput for () {
    fn into_before_result(self) -> HandlerResult<()> {
        Ok(())
    }
}

impl<E> BeforeOutput for Result<(), E>
where
    E: Into<HandlerError>,
{
    fn into_before_result(self) -> HandlerResult<()> {
        self.map_err(Into::into)
    }
}

/// Hook executed before a matched handler.
///
/// Infallible hooks may return `()` directly. Fallible hooks return
/// `HandlerResult<()>` or another result whose error explicitly converts into
/// [`HandlerError`].
pub trait Before<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Executes before handler argument extraction and invocation.
    fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<()>>;
}

impl<S, F, Fut, Output> Before<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Output> + Send + 'static,
    Output: BeforeOutput + 'static,
{
    fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<()>> {
        let future = (self)(context);
        Box::pin(async move { future.await.into_before_result() })
    }
}

#[doc(hidden)]
pub trait AfterOutput {
    fn into_after_result(self) -> HandlerResult<Outcome>;
}

impl AfterOutput for Outcome {
    fn into_after_result(self) -> HandlerResult<Outcome> {
        Ok(self)
    }
}

impl<E> AfterOutput for Result<Outcome, E>
where
    E: Into<HandlerError>,
{
    fn into_after_result(self) -> HandlerResult<Outcome> {
        self.map_err(Into::into)
    }
}

/// Hook executed after a matched handler.
///
/// After hooks receive the produced effects and may observe or transform them.
/// Infallible hooks return [`Outcome`] directly; fallible hooks return
/// `HandlerResult<Outcome>`.
pub trait After<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Observes or transforms the effects produced by a matched handler.
    fn call(
        &self,
        context: Context<S>,
        outcome: Outcome,
    ) -> BoxFuture<'static, HandlerResult<Outcome>>;
}

impl<S, F, Fut, Output> After<S> for F
where
    S: Send + Sync + 'static,
    F: Fn(Context<S>, Outcome) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Output> + Send + 'static,
    Output: AfterOutput + 'static,
{
    fn call(
        &self,
        context: Context<S>,
        outcome: Outcome,
    ) -> BoxFuture<'static, HandlerResult<Outcome>> {
        let future = (self)(context, outcome);
        Box::pin(async move { future.await.into_after_result() })
    }
}

pub(crate) type SharedGuard<S> = Arc<dyn Guard<S>>;
pub(crate) type SharedBefore<S> = Arc<dyn Before<S>>;
pub(crate) type SharedAfter<S> = Arc<dyn After<S>>;
