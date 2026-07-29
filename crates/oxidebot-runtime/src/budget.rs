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

    pub(crate) fn can_fit(&self, bytes: usize) -> bool {
        bytes <= self.max_bytes && bytes <= u32::MAX as usize
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

/// Queue admission split into a normal partition and a small high-priority
/// reserve. Normal traffic cannot consume the reserve, while high-priority
/// traffic may use either partition. The sum of both partitions never exceeds
/// the caller-provided item or byte budget.
#[derive(Clone)]
pub(crate) struct PriorityQueueLimiter {
    normal: QueueLimiter,
    reserved_high: Option<QueueLimiter>,
}

impl PriorityQueueLimiter {
    pub(crate) fn new(total: QueueBudget, max_item_bytes: usize) -> Self {
        debug_assert!(max_item_bytes > 0);
        debug_assert!(max_item_bytes <= total.max_bytes / 2);
        let reserve_items = if total.max_items >= 2 {
            (total.max_items / 16)
                .max(1)
                .min(total.max_items.saturating_sub(1))
        } else {
            0
        };
        let reserve_bytes = if reserve_items > 0 { max_item_bytes } else { 0 };
        let reserved_high = (reserve_items > 0)
            .then(|| QueueLimiter::new(QueueBudget::new(reserve_items, reserve_bytes)));
        let normal = QueueLimiter::new(QueueBudget::new(
            total.max_items.saturating_sub(reserve_items).max(1),
            total.max_bytes.saturating_sub(reserve_bytes).max(1),
        ));
        Self {
            normal,
            reserved_high,
        }
    }

    pub(crate) async fn acquire_normal(
        &self,
        bytes: usize,
    ) -> Result<QueueLease, QueueAcquireError> {
        self.normal.acquire(bytes).await
    }

    pub(crate) fn try_acquire_normal(&self, bytes: usize) -> Result<QueueLease, QueueAcquireError> {
        self.normal.try_acquire(bytes)
    }

    pub(crate) async fn acquire_high(&self, bytes: usize) -> Result<QueueLease, QueueAcquireError> {
        let Some(reserved) = &self.reserved_high else {
            return self.normal.acquire(bytes).await;
        };
        match (reserved.can_fit(bytes), self.normal.can_fit(bytes)) {
            (false, false) => Err(QueueAcquireError::TooLarge),
            (true, false) => reserved.acquire(bytes).await,
            (false, true) => self.normal.acquire(bytes).await,
            (true, true) => {
                // High-priority work first consumes ordinary capacity. The
                // dedicated reserve remains available for an interaction that
                // arrives after normal traffic has saturated the queue.
                if let Ok(lease) = self.normal.try_acquire(bytes) {
                    return Ok(lease);
                }
                if let Ok(lease) = reserved.try_acquire(bytes) {
                    return Ok(lease);
                }
                tokio::select! {
                    lease = self.normal.acquire(bytes) => lease,
                    lease = reserved.acquire(bytes) => lease,
                }
            }
        }
    }

    pub(crate) fn try_acquire_high(&self, bytes: usize) -> Result<QueueLease, QueueAcquireError> {
        let Some(reserved) = &self.reserved_high else {
            return self.normal.try_acquire(bytes);
        };
        let normal_error = match self.normal.try_acquire(bytes) {
            Ok(lease) => return Ok(lease),
            Err(error) => error,
        };
        let reserved_error = match reserved.try_acquire(bytes) {
            Ok(lease) => return Ok(lease),
            Err(error) => error,
        };
        match (reserved_error, normal_error) {
            (QueueAcquireError::Closed, _) | (_, QueueAcquireError::Closed) => {
                Err(QueueAcquireError::Closed)
            }
            (QueueAcquireError::TooLarge, QueueAcquireError::TooLarge) => {
                Err(QueueAcquireError::TooLarge)
            }
            _ => Err(QueueAcquireError::Full),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn high_priority_queue_admission_survives_normal_saturation() {
        let limiter = PriorityQueueLimiter::new(QueueBudget::new(8, 64 * 1024), 16 * 1024);
        let mut normal = Vec::new();
        for _ in 0..7 {
            normal.push(
                limiter
                    .try_acquire_normal(1)
                    .expect("normal partition has seven item permits"),
            );
        }
        assert_eq!(
            limiter.try_acquire_normal(1).map(|_| ()),
            Err(QueueAcquireError::Full)
        );
        let high = limiter
            .try_acquire_high(1)
            .expect("reserved high-priority partition remains available");
        drop(high);
        drop(normal);
    }

    #[tokio::test]
    async fn tiny_queues_still_reserve_one_high_priority_slot() {
        let limiter = PriorityQueueLimiter::new(QueueBudget::new(2, 32 * 1024), 16 * 1024);
        let normal = limiter.try_acquire_normal(1).expect("one normal item fits");
        assert_eq!(
            limiter.try_acquire_normal(1).map(|_| ()),
            Err(QueueAcquireError::Full)
        );
        let high = limiter
            .try_acquire_high(16 * 1024)
            .expect("one high-priority item remains reserved");
        drop(high);
        drop(normal);
    }

    #[tokio::test]
    async fn configured_max_item_fits_normal_and_high_partitions() {
        let limiter = PriorityQueueLimiter::new(QueueBudget::new(16, 128 * 1024), 32 * 1024);
        let normal = limiter
            .try_acquire_normal(32 * 1024)
            .expect("maximum normal item fits the normal partition");
        drop(normal);
        let high = limiter
            .try_acquire_high(32 * 1024)
            .expect("maximum high-priority item fits the reserve");
        drop(high);
    }
}
