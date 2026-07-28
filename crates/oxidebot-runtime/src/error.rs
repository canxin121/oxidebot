use oxidebot_core::event::kernel::DispatchValidationError;
use oxidebot_core::message::ModelError;
use std::{sync::Arc, time::Duration};
use thiserror::Error;

/// Broad adapter failure category used to distinguish expected shutdown from
/// transport/decoder failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterErrorKind {
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct AdapterError {
    kind: AdapterErrorKind,
    message: Arc<str>,
}
impl AdapterError {
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            kind: AdapterErrorKind::Failed,
            message: message.into(),
        }
    }

    pub fn cancelled(message: impl Into<Arc<str>>) -> Self {
        Self {
            kind: AdapterErrorKind::Cancelled,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> AdapterErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self.kind, AdapterErrorKind::Cancelled)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct DecodeError {
    message: Arc<str>,
}
impl DecodeError {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformErrorKind {
    Temporary,
    RateLimited,
    Timeout,
    Permanent,
    Unsupported,
    NotFound,
}

#[derive(Clone, Debug, Error)]
#[error("{message}")]
pub struct PlatformError {
    pub kind: PlatformErrorKind,
    pub message: Arc<str>,
    pub retry_after: Option<Duration>,
}
impl PlatformError {
    pub fn new(kind: PlatformErrorKind, message: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: None,
        }
    }
    #[must_use]
    pub fn retry_after(mut self, value: Duration) -> Self {
        self.retry_after = Some(value);
        self
    }
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

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("bot command queue is closed")]
    Closed,
    #[error("bot command queue is full")]
    Full,
    #[error("bot command payload exceeds its byte budget")]
    PayloadTooLarge,
    #[error("bot command exceeded its total deadline")]
    DeadlineExceeded,
    #[error("bot command sequence space is exhausted")]
    SequenceExhausted,
    #[error("bot command caller was cancelled")]
    Cancelled,
    #[error("adapter returned an unexpected command result")]
    UnexpectedResult,
    #[error("interaction service is unsupported")]
    InteractionsUnsupported,
    #[error("native API service is unsupported")]
    NativeUnsupported,
    #[error("the adapter does not provide the OxideBot API")]
    ApiUnsupported,
    #[error("platform service panicked")]
    ServicePanicked,
    #[error("command target belongs to another bot")]
    WrongBot,
    #[error("platform-native command data belongs to another platform")]
    WrongPlatform,
    #[error("command model is invalid: {0}")]
    InvalidModel(String),
    #[error(transparent)]
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

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SessionError {
    #[error("session registry is closed")]
    Closed,
    #[error("session capacity is full")]
    Full,
    #[error("an equivalent exclusive session is already registered")]
    Occupied,
    #[error("session timed out")]
    Timeout,
    #[error("session was cancelled")]
    Cancelled,
    #[error("session timeout must be non-zero and fit the monotonic clock")]
    InvalidTimeout,
    #[error("session registration sequence space is exhausted")]
    SequenceExhausted,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct ServiceError {
    message: Arc<str>,
}
impl ServiceError {
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum HandlerError {
    /// A deliberately user-facing failure. Only this variant is replied to.
    #[error("{0}")]
    User(Arc<str>),
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("could not parse session response: {0}")]
    Parse(String),
    #[error("bot API call failed: {0}")]
    Api(String),
    #[error("handler failed: {0}")]
    Internal(String),
    #[error("handler timed out")]
    Timeout,
}

impl HandlerError {
    #[must_use]
    pub fn user(message: impl Into<Arc<str>>) -> Self {
        Self::User(message.into())
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    #[must_use]
    pub fn user_message(&self) -> Option<&str> {
        match self {
            Self::User(message) => Some(message),
            _ => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("at least one adapter must be registered")]
    NoAdapters,
    #[error("duplicate bot identity: {0}")]
    DuplicateBot(String),
    #[error("invalid bot descriptor: {0}")]
    InvalidBot(String),
    #[error("too many adapters to assign bot slots")]
    TooManyBots,
    #[error("invalid route: {0}")]
    InvalidRoute(String),
    #[error("run_to_completion requires finite adapters")]
    PersistentAdapterInFiniteRun,
    #[error("invalid runtime configuration: {0}")]
    InvalidConfig(&'static str),
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error("runtime channel failure: {0}")]
    Channel(&'static str),
    #[error("event exceeds the configured executor envelope: {0}")]
    EventTooLarge(String),
    #[error("runtime task failed: {0}")]
    Join(String),
    #[error("shutdown timed out while draining {0}")]
    ShutdownTimeout(&'static str),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
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
        self.map_err(|error| {
            HandlerError::Internal(format!("{}: {error}", operation.as_ref()))
        })
    }

    fn api(self) -> HandlerResult<T> {
        self.map_err(|error| HandlerError::Api(error.to_string()))
    }

    fn api_context(self, operation: impl AsRef<str>) -> HandlerResult<T> {
        self.map_err(|error| {
            HandlerError::Api(format!("{}: {error}", operation.as_ref()))
        })
    }

    fn user(self, message: impl Into<Arc<str>>) -> HandlerResult<T> {
        self.map_err(|_| HandlerError::User(message.into()))
    }
}

/// Safe conversion helpers for optional application data.
pub trait OptionExt<T>: Sized {
    fn user_or(self, message: impl Into<Arc<str>>) -> HandlerResult<T>;
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
