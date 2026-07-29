use oxidebot_core::event::kernel::DispatchValidationError;
use oxidebot_core::message::ModelError;
use oxidebot_core::PartialDeliveryError;
use std::{sync::Arc, time::Duration};
use thiserror::Error;

/// Broad adapter failure category used to distinguish expected shutdown from
/// transport/decoder failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterErrorKind {
    /// Adapter stopped because of a failure.
    Failed,
    /// Adapter stopped as part of expected cancellation or shutdown.
    Cancelled,
}

/// Adapter startup, ingress, or transport failure.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct AdapterError {
    kind: AdapterErrorKind,
    message: Arc<str>,
}
impl AdapterError {
    /// Creates a non-cancellation adapter failure.
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            kind: AdapterErrorKind::Failed,
            message: message.into(),
        }
    }

    /// Creates an expected-cancellation adapter failure.
    pub fn cancelled(message: impl Into<Arc<str>>) -> Self {
        Self {
            kind: AdapterErrorKind::Cancelled,
            message: message.into(),
        }
    }

    /// Returns the broad adapter failure category.
    #[must_use]
    pub const fn kind(&self) -> AdapterErrorKind {
        self.kind
    }

    /// Returns whether this error represents expected cancellation.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self.kind, AdapterErrorKind::Cancelled)
    }
}

/// Failure while decoding a platform frame into OxideBot events.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct DecodeError {
    message: Arc<str>,
}
impl DecodeError {
    /// Creates a decode error from human-readable detail.
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
impl From<DecodeError> for AdapterError {
    fn from(value: DecodeError) -> Self {
        Self::new(value.message)
    }
}
impl From<DispatchValidationError> for AdapterError {
    fn from(value: DispatchValidationError) -> Self {
        Self::new(value.to_string())
    }
}

/// Classification of a platform service failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformErrorKind {
    /// Transient platform or transport failure.
    Temporary,
    /// Platform rate limit was exceeded.
    RateLimited,
    /// Platform call timed out.
    Timeout,
    /// Non-retryable platform failure.
    Permanent,
    /// Request was invalid.
    InvalidRequest,
    /// Requested platform feature is unavailable.
    Unsupported,
    /// Requested platform resource was not found.
    NotFound,
}

/// Classified failure returned by a platform service implementation.
#[derive(Clone, Debug, Error)]
#[error("{message}")]
pub struct PlatformError {
    /// Failure category used by retry scheduling.
    pub kind: PlatformErrorKind,
    /// Human-readable platform failure detail.
    pub message: Arc<str>,
    /// Server-supplied retry delay, if any.
    pub retry_after: Option<Duration>,
}
impl PlatformError {
    /// Creates a classified platform error with no retry delay.
    pub fn new(kind: PlatformErrorKind, message: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: None,
        }
    }
    /// Adds a server-supplied retry delay.
    #[must_use]
    pub fn retry_after(mut self, value: Duration) -> Self {
        self.retry_after = Some(value);
        self
    }
    /// Returns whether scheduler retry is allowed for this error.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            PlatformErrorKind::Temporary
                | PlatformErrorKind::RateLimited
                | PlatformErrorKind::Timeout
        )
    }
}

/// Error returned by a runtime command sent to a bot service.
#[derive(Debug, Error)]
pub enum CommandError {
    /// Bot command queue is closed.
    #[error("bot command queue is closed")]
    Closed,
    #[error("bot command queue is full")]
    /// Bot command queue cannot accept more commands.
    Full,
    /// Command payload exceeds the configured byte budget.
    #[error("bot command payload exceeds its byte budget")]
    PayloadTooLarge,
    #[error("bot command exceeded its total deadline")]
    /// Command exceeded its total deadline.
    DeadlineExceeded,
    /// Command sequence identifiers are exhausted.
    #[error("bot command sequence space is exhausted")]
    SequenceExhausted,
    #[error("bot command caller was cancelled")]
    /// Command caller was cancelled.
    Cancelled,
    /// Adapter returned a result for a different command shape.
    #[error("adapter returned an unexpected command result")]
    UnexpectedResult,
    #[error("interaction service is unsupported")]
    /// Adapter has no interaction service.
    InteractionsUnsupported,
    /// Adapter has no platform-native API service.
    #[error("native API service is unsupported")]
    NativeUnsupported,
    #[error("the adapter does not provide the OxideBot API")]
    /// Adapter does not expose the portable OxideBot API.
    ApiUnsupported,
    /// Platform service panicked during command processing.
    #[error("platform service panicked")]
    ServicePanicked,
    #[error(transparent)]
    /// Delivery partly succeeded before a physical message failed.
    PartialDelivery(#[from] PartialDeliveryError),
    /// Command target belongs to another registered bot.
    #[error("command target belongs to another bot")]
    WrongBot,
    #[error("platform-native command data belongs to another platform")]
    /// Platform-native data belongs to a different platform.
    WrongPlatform,
    /// Command model failed validation.
    #[error("command model is invalid: {0}")]
    InvalidModel(String),
    #[error(transparent)]
    /// Classified platform service failure.
    Platform(#[from] PlatformError),
}

impl From<ModelError> for CommandError {
    fn from(value: ModelError) -> Self {
        match value {
            ModelError::WrongBot => Self::WrongBot,
            ModelError::WrongPlatform => Self::WrongPlatform,
            other => Self::InvalidModel(other.to_string()),
        }
    }
}

/// Error returned while managing a dialogue session.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SessionError {
    /// Session registry is closed.
    #[error("session registry is closed")]
    Closed,
    #[error("session capacity is full")]
    /// Session registry capacity is exhausted.
    Full,
    /// Equivalent exclusive session already exists.
    #[error("an equivalent exclusive session is already registered")]
    Occupied,
    #[error("session timed out")]
    /// Session deadline elapsed.
    Timeout,
    /// Session was cancelled.
    #[error("session was cancelled")]
    Cancelled,
    #[error("session timeout must be non-zero and fit the monotonic clock")]
    /// Requested session timeout is invalid.
    InvalidTimeout,
    /// Session registration sequence identifiers are exhausted.
    #[error("session registration sequence space is exhausted")]
    SequenceExhausted,
}

/// Failure from a runtime-provided service that is not a platform API call.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct ServiceError {
    message: Arc<str>,
}
impl ServiceError {
    /// Creates a service error from human-readable detail.
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Failure returned while running a handler, middleware, or delivery pipeline.
#[derive(Debug, Error)]
pub enum HandlerError {
    /// A deliberately user-facing failure. Only this variant is replied to.
    #[error("{0}")]
    User(Arc<str>),
    #[error(transparent)]
    /// Command service failure.
    Command(#[from] CommandError),
    /// Dialogue session failure.
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("could not parse session response: {0}")]
    /// Session response could not be parsed.
    Parse(String),
    /// Non-user-facing bot API failure.
    #[error("bot API call failed: {0}")]
    Api(String),
    #[error("handler failed: {0}")]
    /// Non-user-facing internal application failure.
    Internal(String),
    /// Handler exceeded its execution deadline.
    #[error("handler timed out")]
    Timeout,
    #[error("delivery middleware panicked")]
    /// Delivery middleware panicked; its failure was isolated.
    DeliveryPanicked,
    /// Mutation partly succeeded before a later operation failed.
    #[error(transparent)]
    PartialMutation(#[from] crate::PartialMutationError),
}

impl HandlerError {
    /// Creates an intentional user-visible handler failure.
    #[must_use]
    pub fn user(message: impl Into<Arc<str>>) -> Self {
        Self::User(message.into())
    }

    /// Creates a non-user-facing internal handler failure.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Returns intentional user-visible text, if this is such a failure.
    #[must_use]
    pub fn user_message(&self) -> Option<&str> {
        match self {
            Self::User(message) => Some(message),
            _ => None,
        }
    }
}

/// Error while building an immutable OxideBot application.
#[derive(Debug, Error)]
pub enum BuildError {
    /// No adapter was registered.
    #[error("at least one adapter must be registered")]
    NoAdapters,
    #[error("duplicate bot identity: {0}")]
    /// Multiple adapters use the same bot identity.
    DuplicateBot(String),
    /// Adapter descriptor was invalid.
    #[error("invalid bot descriptor: {0}")]
    InvalidBot(String),
    #[error("too many adapters to assign bot slots")]
    /// There are too many adapters to assign dense bot slots.
    TooManyBots,
    /// A handler route was invalid.
    #[error("invalid route: {0}")]
    InvalidRoute(String),
    #[error("run_to_completion requires finite adapters")]
    /// Finite execution was requested with a persistent adapter.
    PersistentAdapterInFiniteRun,
    /// Runtime configuration violates its invariant.
    #[error("invalid runtime configuration: {0}")]
    InvalidConfig(&'static str),
}

/// Top-level runtime failure.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Application build failure.
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    /// Adapter task failure.
    Adapter(#[from] AdapterError),
    /// Runtime-provided service failure.
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error("runtime channel failure: {0}")]
    /// Internal runtime channel failure.
    Channel(&'static str),
    /// Event exceeds executor byte envelope.
    #[error("event exceeds the configured executor envelope: {0}")]
    EventTooLarge(String),
    #[error("runtime task failed: {0}")]
    /// Runtime task join failure.
    Join(String),
    /// Shutdown drain exceeded its timeout.
    #[error("shutdown timed out while draining {0}")]
    ShutdownTimeout(&'static str),
}

/// Standard result for runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;
/// Standard result returned by a handler.
pub type HandlerResult<T = crate::Outcome> = std::result::Result<T, HandlerError>;

/// Safe, explicit conversion helpers for application and platform results.
///
/// These helpers reduce repetitive `map_err` blocks without ever turning an
/// arbitrary internal error into a user-visible chat reply.
pub trait ResultExt<T>: Sized {
    /// Records an application failure as an internal handler error with useful
    /// operation context. The resulting text is logged by the runtime, not sent
    /// to the user.
    fn internal(self, operation: impl AsRef<str>) -> HandlerResult<T>;

    /// Marks a platform/API operation failure. The error remains non-user-facing.
    fn api(self) -> HandlerResult<T>;

    /// Marks a platform/API failure and records the operation that failed.
    fn api_context(self, operation: impl AsRef<str>) -> HandlerResult<T>;

    /// Replaces an expected application failure with an intentional user-facing
    /// message while keeping the original error out of the reply.
    fn user(self, message: impl Into<Arc<str>>) -> HandlerResult<T>;
}

impl<T, E> ResultExt<T> for std::result::Result<T, E>
where
    E: std::fmt::Display,
{
    fn internal(self, operation: impl AsRef<str>) -> HandlerResult<T> {
        self.map_err(|error| HandlerError::Internal(format!("{}: {error}", operation.as_ref())))
    }

    fn api(self) -> HandlerResult<T> {
        self.map_err(|error| HandlerError::Api(error.to_string()))
    }

    fn api_context(self, operation: impl AsRef<str>) -> HandlerResult<T> {
        self.map_err(|error| HandlerError::Api(format!("{}: {error}", operation.as_ref())))
    }

    fn user(self, message: impl Into<Arc<str>>) -> HandlerResult<T> {
        self.map_err(|_| HandlerError::User(message.into()))
    }
}

/// Safe conversion helpers for optional application data.
pub trait OptionExt<T>: Sized {
    /// Converts `None` into an intentional user-visible failure.
    fn user_or(self, message: impl Into<Arc<str>>) -> HandlerResult<T>;
    /// Converts `None` into a non-user-facing internal failure.
    fn internal_or(self, operation: impl Into<String>) -> HandlerResult<T>;
}

impl<T> OptionExt<T> for Option<T> {
    fn user_or(self, message: impl Into<Arc<str>>) -> HandlerResult<T> {
        self.ok_or_else(|| HandlerError::User(message.into()))
    }

    fn internal_or(self, operation: impl Into<String>) -> HandlerResult<T> {
        self.ok_or_else(|| HandlerError::Internal(operation.into()))
    }
}
