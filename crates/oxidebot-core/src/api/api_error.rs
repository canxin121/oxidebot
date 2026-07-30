//! Typed failures shared by portable and platform-native adapter calls.

use std::{sync::Arc, time::Duration};

use thiserror::Error;

use super::platform::UnsupportedPlatformApiError;
use crate::{
    capability::UnsupportedFeatureError,
    interaction::UnsupportedInteractionError,
    source::message::{DeliveryPlanningError, PartialDeliveryError},
};

/// Result returned by every portable or platform-native adapter call.
pub type CallResult<T> = std::result::Result<T, CallError>;

/// Typed adapter-call failure used by the runtime scheduler and applications.
///
/// Adapters must classify transport failures explicitly. This prevents an
/// unknown erased error from silently becoming a permanent failure and makes
/// retry, rate-limit, partial-delivery, and unsupported-feature handling
/// deterministic across every API method.
#[derive(Debug, Error)]
pub enum CallError {
    /// A transient transport or remote-service failure that may be retried.
    #[error("temporary adapter failure: {message}")]
    Temporary {
        /// Human-readable failure detail supplied by the adapter.
        message: Arc<str>,
    },
    /// The remote platform rejected the request because its rate limit is exhausted.
    #[error("adapter rate limited the call: {message}")]
    RateLimited {
        /// Human-readable failure detail supplied by the adapter.
        message: Arc<str>,
        /// Suggested delay before another attempt, when supplied by the platform.
        retry_after: Option<Duration>,
    },
    /// The adapter did not complete the call before its deadline.
    #[error("adapter call timed out: {message}")]
    Timeout {
        /// Human-readable failure detail supplied by the adapter.
        message: Arc<str>,
    },
    /// The request is invalid and retrying it unchanged cannot succeed.
    #[error("adapter rejected the request: {message}")]
    InvalidRequest {
        /// Human-readable validation failure detail.
        message: Arc<str>,
    },
    /// The requested platform resource does not exist.
    #[error("adapter could not find the requested resource: {message}")]
    NotFound {
        /// Human-readable missing-resource detail.
        message: Arc<str>,
    },
    /// The adapter does not implement a requested capability.
    #[error("adapter does not support {feature}")]
    Unsupported {
        /// Name of the unavailable feature.
        feature: Arc<str>,
    },
    /// A non-retryable adapter or remote-service failure.
    #[error("adapter call failed permanently: {message}")]
    Permanent {
        /// Human-readable failure detail supplied by the adapter.
        message: Arc<str>,
    },
    /// Portable delivery planning could not produce a valid physical message plan.
    #[error(transparent)]
    Planning(#[from] DeliveryPlanningError),
    /// A multipart delivery failed after one or more physical messages succeeded.
    #[error(transparent)]
    PartialDelivery(#[from] PartialDeliveryError),
}

impl CallError {
    /// Creates a retryable temporary failure.
    #[must_use]
    pub fn temporary(message: impl Into<Arc<str>>) -> Self {
        Self::Temporary {
            message: message.into(),
        }
    }

    /// Creates a rate-limit failure with an optional server-supplied delay.
    #[must_use]
    pub fn rate_limited(message: impl Into<Arc<str>>, retry_after: Option<Duration>) -> Self {
        Self::RateLimited {
            message: message.into(),
            retry_after,
        }
    }

    /// Creates a retryable timeout failure.
    #[must_use]
    pub fn timeout(message: impl Into<Arc<str>>) -> Self {
        Self::Timeout {
            message: message.into(),
        }
    }

    /// Creates a non-retryable request-validation failure.
    #[must_use]
    pub fn invalid_request(message: impl Into<Arc<str>>) -> Self {
        Self::InvalidRequest {
            message: message.into(),
        }
    }

    /// Creates a missing-resource failure.
    #[must_use]
    pub fn not_found(message: impl Into<Arc<str>>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    /// Creates an unsupported-capability failure.
    #[must_use]
    pub fn unsupported(feature: impl Into<Arc<str>>) -> Self {
        Self::Unsupported {
            feature: feature.into(),
        }
    }

    /// Creates a non-retryable permanent failure.
    #[must_use]
    pub fn permanent(message: impl Into<Arc<str>>) -> Self {
        Self::Permanent {
            message: message.into(),
        }
    }

    /// Returns whether the scheduler may retry this error.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Temporary { .. } | Self::RateLimited { .. } | Self::Timeout { .. }
        )
    }

    /// Returns the server-supplied rate-limit delay, if any.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl From<UnsupportedFeatureError> for CallError {
    fn from(value: UnsupportedFeatureError) -> Self {
        Self::unsupported(value.feature)
    }
}

impl From<UnsupportedInteractionError> for CallError {
    fn from(value: UnsupportedInteractionError) -> Self {
        Self::unsupported(value.feature)
    }
}

impl From<UnsupportedPlatformApiError> for CallError {
    fn from(value: UnsupportedPlatformApiError) -> Self {
        Self::unsupported(format!("platform-native API method {:?}", value.method))
    }
}
