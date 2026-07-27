use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

/// Maximum retained items and bytes for one bounded queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueBudget {
    /// Maximum queued items.
    pub max_items: usize,
    /// Maximum estimated retained bytes.
    pub max_bytes: usize,
}

impl QueueBudget {
    /// Creates a queue budget.
    #[must_use]
    pub const fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            max_items,
            max_bytes,
        }
    }
}

/// Behavior when a bounded execution queue cannot admit more work.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OverloadPolicy {
    /// Wait for capacity and preserve every accepted event.
    #[default]
    Block,
    /// Discard the newest item and increment the corresponding metric.
    DropNewest,
}

#[derive(Clone)]
pub(crate) struct QueueLimiter {
    items: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    max_bytes: usize,
}

impl QueueLimiter {
    pub(crate) fn new(budget: QueueBudget) -> Self {
        Self {
            items: Arc::new(Semaphore::new(budget.max_items)),
            bytes: Arc::new(Semaphore::new(budget.max_bytes)),
            max_bytes: budget.max_bytes,
        }
    }

    pub(crate) async fn acquire(&self, bytes: usize) -> Result<QueueLease, QueueAcquireError> {
        let bytes = self.checked_bytes(bytes)?;
        let item = Arc::clone(&self.items)
            .acquire_owned()
            .await
            .map_err(|_| QueueAcquireError::Closed)?;
        let byte = Arc::clone(&self.bytes)
            .acquire_many_owned(bytes)
            .await
            .map_err(|_| QueueAcquireError::Closed)?;
        Ok(QueueLease {
            _item: item,
            _bytes: byte,
        })
    }

    pub(crate) fn try_acquire(&self, bytes: usize) -> Result<QueueLease, QueueAcquireError> {
        let bytes = self.checked_bytes(bytes)?;
        let item = Arc::clone(&self.items)
            .try_acquire_owned()
            .map_err(map_try_error)?;
        let byte = Arc::clone(&self.bytes)
            .try_acquire_many_owned(bytes)
            .map_err(map_try_error)?;
        Ok(QueueLease {
            _item: item,
            _bytes: byte,
        })
    }

    fn checked_bytes(&self, bytes: usize) -> Result<u32, QueueAcquireError> {
        if bytes > self.max_bytes || bytes > u32::MAX as usize {
            return Err(QueueAcquireError::TooLarge);
        }
        Ok(bytes.max(1) as u32)
    }
}

fn map_try_error(error: TryAcquireError) -> QueueAcquireError {
    match error {
        TryAcquireError::NoPermits => QueueAcquireError::Full,
        TryAcquireError::Closed => QueueAcquireError::Closed,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueueAcquireError {
    Closed,
    Full,
    TooLarge,
}

#[derive(Debug)]
pub(crate) struct QueueLease {
    _item: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}
