use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

/// Maximum retained items and bytes for one bounded queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueBudget {
    pub max_items: usize,
    pub max_bytes: usize,
}

impl QueueBudget {
    #[must_use]
    pub const fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            max_items,
            max_bytes,
        }
    }
}

/// Behavior when a bounded queue cannot admit more work.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OverloadPolicy {
    #[default]
    Block,
    DropNewest,
}

/// Combined item/byte limiter. Callers must acquire all hierarchical limiters
/// in the same local-then-global order to avoid cross-resource deadlocks.
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
        let byte = Arc::clone(&self.bytes)
            .acquire_many_owned(bytes)
            .await
            .map_err(|_| QueueAcquireError::Closed)?;
        let item = match Arc::clone(&self.items).acquire_owned().await {
            Ok(item) => item,
            Err(_) => {
                drop(byte);
                return Err(QueueAcquireError::Closed);
            }
        };
        Ok(QueueLease {
            _item: item,
            _bytes: byte,
        })
    }

    pub(crate) fn try_acquire(&self, bytes: usize) -> Result<QueueLease, QueueAcquireError> {
        let bytes = self.checked_bytes(bytes)?;
        let byte = Arc::clone(&self.bytes)
            .try_acquire_many_owned(bytes)
            .map_err(map_try_error)?;
        let item = match Arc::clone(&self.items).try_acquire_owned() {
            Ok(item) => item,
            Err(error) => {
                drop(byte);
                return Err(map_try_error(error));
            }
        };
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

/// Two-level lease used to enforce process-wide and per-bot bounds.
#[derive(Debug)]
pub(crate) struct HierarchicalLease {
    _local: QueueLease,
    _global: QueueLease,
}

impl HierarchicalLease {
    pub(crate) fn new(local: QueueLease, global: QueueLease) -> Self {
        Self {
            _local: local,
            _global: global,
        }
    }
}
