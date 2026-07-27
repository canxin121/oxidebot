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

/// TTL-, item-, and byte-bounded dedupe history.
///
/// Entries are kept in an indexed intrusive list rather than an ordered map:
/// duplicate lookup, insertion, expiry, per-bot eviction, and global eviction
/// are all average O(1). The only heap allocations after warm-up are HashMap
/// growths; removed node slots are reused through `free`.
pub(crate) struct DedupeCache {
    ttl: Duration,
    max_entries: usize,
    max_bytes: usize,
    per_bot_max_entries: usize,
    per_bot_max_bytes: usize,
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
        Self {
            ttl,
            max_entries: total_capacity,
            max_bytes: total_max_bytes,
            per_bot_max_entries: total_capacity.div_ceil(bot_count).max(1),
            per_bot_max_bytes: total_max_bytes.div_ceil(bot_count).max(1),
            active_entries: 0,
            retained_bytes: 0,
            bots: (0..bot_count).map(|_| BotList::default()).collect(),
            lookup: HashMap::with_capacity(total_capacity),
            nodes: Vec::with_capacity(total_capacity),
            free: Vec::new(),
            global_head: None,
            global_tail: None,
        }
    }

    /// Inserts one delivery ID and returns `false` while the same ID is still
    /// active for the same bot. Unknown bot slots are rejected as duplicates;
    /// validated runtimes never emit them.
    pub(crate) fn insert(&mut self, bot: BotSlot, id: EventId, now: Instant) -> bool {
        self.prune_expired(now);
        let bot_index = bot.0 as usize;
        if bot_index >= self.bots.len() {
            return false;
        }
        if self.lookup.contains_key(&(bot, id.clone())) {
            return false;
        }

        let bytes = id.estimated_bytes().saturating_add(192);
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

        self.enforce_bot_bounds(bot_index);
        self.enforce_global_bounds();
        true
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

    fn enforce_bot_bounds(&mut self, bot_index: usize) {
        while self.bots[bot_index].entries > self.per_bot_max_entries
            || self.bots[bot_index].retained_bytes > self.per_bot_max_bytes
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

        let lookup_key = (node.bot, node.id.clone());
        self.lookup.remove(&lookup_key);
        self.active_entries = self.active_entries.saturating_sub(1);
        self.retained_bytes = self.retained_bytes.saturating_sub(node.bytes);
        self.free.push(index);
    }
}
