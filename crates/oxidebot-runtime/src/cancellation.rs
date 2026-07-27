use tokio_util::sync::CancellationToken;

/// Read-only structured-shutdown signal exposed to handlers, services, and adapters.
#[derive(Clone, Debug)]
pub struct ShutdownSignal(pub(crate) CancellationToken);

impl ShutdownSignal {
    pub(crate) fn new(token: CancellationToken) -> Self {
        Self(token)
    }

    /// Waits until the owning runtime scope starts shutting down.
    pub async fn cancelled(&self) {
        self.0.cancelled().await;
    }

    /// Returns whether shutdown has started.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
