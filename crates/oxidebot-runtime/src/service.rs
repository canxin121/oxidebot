use crate::BotDirectory;
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Facilities shared with one supervised background service.
#[derive(Clone)]
pub struct ServiceContext<S>
where
    S: Send + Sync + 'static,
{
    state: Arc<S>,
    bots: BotDirectory,
    cancellation: CancellationToken,
}

impl<S> ServiceContext<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn new(state: Arc<S>, bots: BotDirectory, cancellation: CancellationToken) -> Self {
        Self {
            state,
            bots,
            cancellation,
        }
    }
    #[must_use]
    pub fn state(&self) -> &S {
        &self.state
    }
    #[must_use]
    pub fn bots(&self) -> &BotDirectory {
        &self.bots
    }
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

/// A long-lived task supervised by the runtime.
#[async_trait]
pub trait Service<S>: Send + Sync + 'static
where
    S: Send + Sync + 'static,
{
    async fn run(&self, context: ServiceContext<S>) -> Result<(), crate::ServiceError>;
}
