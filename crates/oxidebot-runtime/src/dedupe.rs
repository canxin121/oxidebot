use oxidebot_core::{BotSlot, EventId};
use std::collections::{HashSet, VecDeque};

pub(crate) struct DedupeCache {
    capacity: usize,
    seen: HashSet<(BotSlot, EventId)>,
    order: VecDeque<(BotSlot, EventId)>,
}
impl DedupeCache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            seen: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
        }
    }
    pub(crate) fn insert(&mut self, bot: BotSlot, event: EventId) -> bool {
        let key = (bot, event);
        if !self.seen.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        if self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        true
    }
}
