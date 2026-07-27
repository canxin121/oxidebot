use oxidebot_core::{BotSlot, EventId};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
struct BotList {
    head: Option<usize>,
    tail: Option<usize>,
    entries: usize,
    retained_bytes: usize,
}

struct Node {
    bot: BotSlot,
    id: EventId,
    inserted_at: Instant,
    bytes: usize,
    global_previous: Option<usize>,
    global_next: Option<usize>,
    bot_previous: Option<usize>,
    bot_next: Option<usize>,
}

/// Result of committing an accepted event ID to the dedupe history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DedupeCommit {
    Inserted,
    Duplicate,
    /// The ID itself cannot fit in the configured global byte envelope.
    Uncacheable,
}

/// TTL-, item-, and byte-bounded dedupe history.
///
/// Entries are kept in indexed intrusive lists. Duplicate lookup, insertion,
/// expiry, per-bot pressure eviction, and global eviction are average O(1).
/// Per-bot limits are soft: an active bot may borrow unused capacity until the
/// global cache reaches pressure, while the global item/byte limits remain hard.
pub(crate) struct DedupeCache {
    ttl: Duration,
    max_entries: usize,
    max_bytes: usize,
    per_bot_soft_entries: usize,
    per_bot_soft_bytes: usize,
    active_entries: usize,
    retained_bytes: usize,
    bots: Vec<BotList>,
    lookup: HashMap<(BotSlot, EventId), usize>,
    nodes: Vec<Option<Node>>,
    free: Vec<usize>,
    global_head: Option<usize>,
    global_tail: Option<usize>,
}

impl DedupeCache {
    pub(crate) fn new(
        total_capacity: usize,
        total_max_bytes: usize,
        ttl: Duration,
        bot_count: usize,
    ) -> Self {
        let total_capacity = total_capacity.max(1);
        let total_max_bytes = total_max_bytes.max(1);
        let bot_count = bot_count.max(1);

        // Do not trust a huge item count to justify a huge eager allocation when
        // the byte budget is tiny. Grow lazily after a small bounded warm-up.
        let estimated_entry_bytes = std::mem::size_of::<Node>()
            .saturating_add(std::mem::size_of::<((BotSlot, EventId), usize)>())
            .saturating_add(64);
        let initial_capacity = total_capacity
            .min(total_max_bytes / estimated_entry_bytes.max(1))
            .min(4_096);

        Self {
            ttl,
            max_entries: total_capacity,
            max_bytes: total_max_bytes,
            per_bot_soft_entries: total_capacity.div_ceil(bot_count).max(1),
            per_bot_soft_bytes: total_max_bytes.div_ceil(bot_count).max(1),
            active_entries: 0,
            retained_bytes: 0,
            bots: (0..bot_count).map(|_| BotList::default()).collect(),
            lookup: HashMap::with_capacity(initial_capacity),
            nodes: Vec::with_capacity(initial_capacity),
            free: Vec::new(),
            global_head: None,
            global_tail: None,
        }
    }

    /// Returns whether the same bot/event ID is currently remembered.
    pub(crate) fn contains(&mut self, bot: BotSlot, id: &EventId, now: Instant) -> bool {
        self.prune_expired(now);
        self.lookup.contains_key(&(bot, id.clone()))
    }

    /// Commits one successfully consumed, admitted, or intentionally shed event.
    pub(crate) fn commit(&mut self, bot: BotSlot, id: EventId, now: Instant) -> DedupeCommit {
        self.prune_expired(now);
        let bot_index = bot.0 as usize;
        if bot_index >= self.bots.len() {
            return DedupeCommit::Uncacheable;
        }
        if self.lookup.contains_key(&(bot, id.clone())) {
            return DedupeCommit::Duplicate;
        }

        let bytes = id
            .estimated_bytes()
            .saturating_add(std::mem::size_of::<Node>())
            .saturating_add(128);
        if bytes > self.max_bytes {
            return DedupeCommit::Uncacheable;
        }

        let global_previous = self.global_tail;
        let bot_previous = self.bots[bot_index].tail;
        let index = self.allocate(Node {
            bot,
            id: id.clone(),
            inserted_at: now,
            bytes,
            global_previous,
            global_next: None,
            bot_previous,
            bot_next: None,
        });

        if let Some(previous) = self.global_tail {
            self.node_mut(previous).global_next = Some(index);
        } else {
            self.global_head = Some(index);
        }
        self.global_tail = Some(index);

        if let Some(previous) = self.bots[bot_index].tail {
            self.node_mut(previous).bot_next = Some(index);
        } else {
            self.bots[bot_index].head = Some(index);
        }
        {
            let bot_list = &mut self.bots[bot_index];
            bot_list.tail = Some(index);
            bot_list.entries = bot_list.entries.saturating_add(1);
            bot_list.retained_bytes = bot_list.retained_bytes.saturating_add(bytes);
        }

        self.lookup.insert((bot, id), index);
        self.active_entries = self.active_entries.saturating_add(1);
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);

        self.enforce_bot_soft_bounds(bot_index);
        self.enforce_global_bounds();
        DedupeCommit::Inserted
    }

    fn allocate(&mut self, node: Node) -> usize {
        if let Some(index) = self.free.pop() {
            debug_assert!(self.nodes[index].is_none());
            self.nodes[index] = Some(node);
            index
        } else {
            let index = self.nodes.len();
            self.nodes.push(Some(node));
            index
        }
    }

    fn node(&self, index: usize) -> &Node {
        self.nodes[index]
            .as_ref()
            .expect("active dedupe links always reference a node")
    }

    fn node_mut(&mut self, index: usize) -> &mut Node {
        self.nodes[index]
            .as_mut()
            .expect("active dedupe links always reference a node")
    }

    fn prune_expired(&mut self, now: Instant) {
        while let Some(index) = self.global_head {
            let expired = now.saturating_duration_since(self.node(index).inserted_at) >= self.ttl;
            if !expired {
                break;
            }
            self.remove(index);
        }
    }

    fn under_pressure(&self) -> bool {
        self.active_entries.saturating_mul(4) >= self.max_entries.saturating_mul(3)
            || self.retained_bytes.saturating_mul(4) >= self.max_bytes.saturating_mul(3)
    }

    fn enforce_bot_soft_bounds(&mut self, bot_index: usize) {
        if !self.under_pressure() {
            return;
        }
        while self.bots[bot_index].entries > 1
            && (self.bots[bot_index].entries > self.per_bot_soft_entries
                || self.bots[bot_index].retained_bytes > self.per_bot_soft_bytes)
        {
            let Some(index) = self.bots[bot_index].head else {
                break;
            };
            self.remove(index);
        }
    }

    fn enforce_global_bounds(&mut self) {
        while self.active_entries > self.max_entries || self.retained_bytes > self.max_bytes {
            let Some(index) = self.global_head else {
                self.active_entries = 0;
                self.retained_bytes = 0;
                break;
            };
            self.remove(index);
        }
    }

    fn remove(&mut self, index: usize) {
        let Some(node) = self.nodes.get_mut(index).and_then(Option::take) else {
            return;
        };

        if let Some(previous) = node.global_previous {
            self.node_mut(previous).global_next = node.global_next;
        } else {
            self.global_head = node.global_next;
        }
        if let Some(next) = node.global_next {
            self.node_mut(next).global_previous = node.global_previous;
        } else {
            self.global_tail = node.global_previous;
        }

        let bot_index = node.bot.0 as usize;
        if let Some(previous) = node.bot_previous {
            self.node_mut(previous).bot_next = node.bot_next;
        } else if let Some(bot) = self.bots.get_mut(bot_index) {
            bot.head = node.bot_next;
        }
        if let Some(next) = node.bot_next {
            self.node_mut(next).bot_previous = node.bot_previous;
        } else if let Some(bot) = self.bots.get_mut(bot_index) {
            bot.tail = node.bot_previous;
        }
        if let Some(bot) = self.bots.get_mut(bot_index) {
            bot.entries = bot.entries.saturating_sub(1);
            bot.retained_bytes = bot.retained_bytes.saturating_sub(node.bytes);
        }

        self.lookup.remove(&(node.bot, node.id.clone()));
        self.active_entries = self.active_entries.saturating_sub(1);
        self.retained_bytes = self.retained_bytes.saturating_sub(node.bytes);
        self.free.push(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidebot_core::{EventId, MAX_SEMANTIC_ID_BYTES};

    fn id(value: &str) -> EventId {
        EventId::new(value).expect("valid test id")
    }

    #[test]
    fn tiny_byte_budget_does_not_eagerly_allocate_from_item_capacity() {
        let cache = DedupeCache::new(usize::MAX / 4, 512, Duration::from_secs(1), 1);
        assert!(cache.nodes.capacity() <= 4_096);
        assert!(cache.lookup.capacity() < 16_384);
    }

    #[test]
    fn uncacheable_entries_are_reported_explicitly() {
        let mut cache = DedupeCache::new(8, 1, Duration::from_secs(60), 1);
        assert_eq!(
            cache.commit(BotSlot(0), id("event"), Instant::now()),
            DedupeCommit::Uncacheable
        );
        assert!(!cache.contains(BotSlot(0), &id("event"), Instant::now()));
    }

    #[test]
    fn active_bot_can_borrow_unused_soft_quota() {
        let mut cache = DedupeCache::new(
            16,
            16 * (MAX_SEMANTIC_ID_BYTES + 256),
            Duration::from_secs(60),
            8,
        );
        let now = Instant::now();
        for index in 0..8 {
            assert_eq!(
                cache.commit(BotSlot(0), id(&format!("event-{index}")), now),
                DedupeCommit::Inserted
            );
        }
        assert!(cache.bots[0].entries > cache.per_bot_soft_entries);
    }
}
