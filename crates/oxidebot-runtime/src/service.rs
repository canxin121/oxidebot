use crate::{BotDirectory, ShutdownSignal};
use async_trait::async_trait;
use std::sync::Arc;

/// Facilities shared with one supervised background service.
#[derive(Clone)]
pub struct ServiceContext<S>
where
    S: Send + Sync + 'static,
{
    state: Arc<S>,
    bots: BotDirectory,
    shutdown: ShutdownSignal,
}

impl<S> ServiceContext<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn new(state: Arc<S>, bots: BotDirectory, shutdown: ShutdownSignal) -> Self {
        Self {
            state,
            bots,
            shutdown,
        }
    }
    /// Returns shared immutable application state.
    #[must_use]
    pub fn state(&self) -> &S {
        &self.state
    }
    /// Returns directory access to registered bots.
    #[must_use]
    pub fn bots(&self) -> &BotDirectory {
        &self.bots
    }
    /// Returns the runtime shutdown signal.
    #[must_use]
    pub fn shutdown(&self) -> &ShutdownSignal {
        &self.shutdown
    }
}

/// A long-lived task supervised by the runtime.
#[async_trait]
pub trait Service<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    /// Runs the supervised service until it finishes, fails, or shutdown is requested.
    async fn run(&self, context: ServiceContext<S>) -> Result<(), crate::ServiceError>;
}
