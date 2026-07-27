use std::{sync::Arc, time::Duration};
use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct AdapterError {
    message: Arc<str>,
}
impl AdapterError {
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformErrorKind {
    Temporary,
    RateLimited,
    Timeout,
    Permanent,
    Unsupported,
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
    #[error("adapter returned an unexpected command result")]
    UnexpectedResult,
    #[error("interaction service is unsupported")]
    InteractionsUnsupported,
    #[error("native API service is unsupported")]
    NativeUnsupported,
    #[error("platform service panicked")]
    ServicePanicked,
    #[error(transparent)]
    Platform(#[from] PlatformError),
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SessionError {
    #[error("session registry is closed")]
    Closed,
    #[error("session capacity is full")]
    Full,
    #[error("an equivalent session is already registered")]
    Occupied,
    #[error("session timed out")]
    Timeout,
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
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("could not parse session response: {0}")]
    Parse(String),
    #[error("handler timed out")]
    Timeout,
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("at least one adapter must be registered")]
    NoAdapters,
    #[error("duplicate bot identity: {0}")]
    DuplicateBot(String),
    #[error("too many adapters to assign bot slots")]
    TooManyBots,
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
    #[error("runtime task failed: {0}")]
    Join(String),
    #[error("shutdown timed out while draining {0}")]
    ShutdownTimeout(&'static str),
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
pub type HandlerResult = std::result::Result<crate::Outcome, HandlerError>;
