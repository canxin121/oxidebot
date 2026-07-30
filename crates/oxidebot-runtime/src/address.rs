//! Explicit outbound addresses and reusable target aliases.
//!
//! Address resolution belongs next to messaging and bot selection, rather than
//! command authoring. The types remain re-exported from the runtime root.

use crate::{BotDirectory, BotHandle, HandlerError};
use oxidebot_core::{
    conversation::{ConversationKind, ConversationRef, MessageTarget},
    source::message::{DeliveryReport, FallbackPolicy, Message},
    BotIdentity, PlatformId,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

/// Selects the bot used when sending to an [`Address`].
#[derive(Clone, Debug)]
pub enum BotSelection {
    /// Use the bot currently processing a handler event.
    Current,
    /// Use the connection with this exact identity.
    Exact(BotIdentity),
    /// Use the sole connected bot for this platform.
    Platform(PlatformId),
}

/// A message target plus deterministic bot-selection policy.
#[derive(Clone, Debug)]
pub struct Address {
    /// Destination conversation and optional recipients.
    pub target: MessageTarget,
    /// Bot selection used to reach the destination.
    pub bot: BotSelection,
}

impl From<MessageTarget> for Address {
    fn from(target: MessageTarget) -> Self {
        Self {
            target,
            bot: BotSelection::Current,
        }
    }
}

impl Address {
    /// Selects a direct-message conversation for `user_id`.
    #[must_use]
    pub fn direct(user_id: impl Into<oxidebot_core::UserId>) -> Self {
        Self {
            target: MessageTarget::new(ConversationRef::direct_user(user_id)),
            bot: BotSelection::Current,
        }
    }

    /// Selects a group conversation for `group_id`.
    #[must_use]
    pub fn group(group_id: impl Into<oxidebot_core::ConversationId>) -> Self {
        Self {
            target: MessageTarget::new(ConversationRef::group(group_id)),
            bot: BotSelection::Current,
        }
    }

    /// Selects a channel conversation for `channel_id`.
    #[must_use]
    pub fn channel(channel_id: impl Into<oxidebot_core::ConversationId>) -> Self {
        Self {
            target: MessageTarget::new(ConversationRef::new(channel_id, ConversationKind::Channel)),
            bot: BotSelection::Current,
        }
    }

    /// Selects a thread conversation beneath `parent`.
    #[must_use]
    pub fn thread(
        thread_id: impl Into<oxidebot_core::ConversationId>,
        parent: ConversationRef,
    ) -> Self {
        Self {
            target: MessageTarget::new(
                ConversationRef::new(thread_id, ConversationKind::Thread).child_of(parent),
            ),
            bot: BotSelection::Current,
        }
    }

    /// Adds explicit recipient IDs to this target.
    #[must_use]
    pub fn recipients(
        mut self,
        recipients: impl IntoIterator<Item = impl Into<oxidebot_core::UserId>>,
    ) -> Self {
        self.target = self.target.recipients(recipients);
        self
    }

    /// Replaces the bot-selection policy for this address.
    #[must_use]
    pub fn through(mut self, bot: BotSelection) -> Self {
        self.bot = bot;
        self
    }
}

impl BotDirectory {
    /// Selects one connected bot, rejecting ambiguous platform-only selections.
    pub fn select(&self, selection: &BotSelection) -> Result<BotHandle, HandlerError> {
        match selection {
            BotSelection::Current => Err(HandlerError::internal(
                "BotSelection::Current requires a handler Context",
            )),
            BotSelection::Exact(identity) => self
                .iter()
                .find(|bot| bot.identity() == identity)
                .cloned()
                .ok_or_else(|| HandlerError::Api("the selected bot is not connected".into())),
            BotSelection::Platform(platform) => {
                let mut matches = self
                    .iter()
                    .filter(|bot| &bot.identity().platform == platform);
                let first = matches.next().cloned();
                if matches.next().is_some() {
                    return Err(HandlerError::Api(
                        "more than one bot matches the platform; select an exact bot".into(),
                    ));
                }
                first.ok_or_else(|| HandlerError::Api("no bot matches the platform".into()))
            }
        }
    }

    /// Sends a canonical message to an explicit address with the requested fallback policy.
    pub async fn send_address(
        &self,
        address: Address,
        message: impl Into<Message>,
        fallback: FallbackPolicy,
    ) -> Result<DeliveryReport, HandlerError> {
        let bot = self.select(&address.bot)?;
        bot.send_outgoing_message_with(address.target, message.into(), fallback)
            .await
            .map_err(HandlerError::from)
    }
}

/// Bounded alias directory for reusable outbound addresses.
#[derive(Clone, Debug)]
pub struct TargetDirectory {
    inner: Arc<RwLock<TargetDirectoryState>>,
}

#[derive(Debug)]
struct TargetDirectoryState {
    capacity: usize,
    max_bytes: usize,
    retained_bytes: usize,
    aliases: BTreeMap<Arc<str>, Address>,
}

impl TargetDirectory {
    /// Creates an alias directory bounded by alias count.
    #[must_use]
    pub fn bounded(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self::bounded_bytes(capacity, capacity.saturating_mul(8 * 1024))
    }

    /// Creates a target directory bounded by aliases and retained address
    /// bytes.
    #[must_use]
    pub fn bounded_bytes(capacity: usize, max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(TargetDirectoryState {
                capacity: capacity.max(1),
                max_bytes: max_bytes.max(1),
                retained_bytes: 0,
                aliases: BTreeMap::new(),
            })),
        }
    }

    /// Inserts or replaces an address alias, enforcing count and byte bounds.
    pub fn insert(&self, alias: impl Into<Arc<str>>, address: Address) -> Result<(), HandlerError> {
        let mut state = self.inner.write().expect("target directory lock poisoned");
        let alias = alias.into();
        if alias.is_empty() || alias.len() > 4 * 1024 {
            return Err(HandlerError::internal(
                "target alias must contain 1..=4096 bytes",
            ));
        }
        if !state.aliases.contains_key(&alias) && state.aliases.len() >= state.capacity {
            return Err(HandlerError::internal("target directory capacity exceeded"));
        }
        let entry_bytes = target_entry_bytes(&alias, &address);
        let previous_bytes = state
            .aliases
            .get(&alias)
            .map_or(0, |previous| target_entry_bytes(&alias, previous));
        let retained_bytes = state
            .retained_bytes
            .saturating_sub(previous_bytes)
            .saturating_add(entry_bytes);
        if retained_bytes > state.max_bytes {
            return Err(HandlerError::internal(
                "target directory byte capacity exceeded",
            ));
        }
        state.aliases.insert(alias, address);
        state.retained_bytes = retained_bytes;
        Ok(())
    }

    /// Resolves an address alias, if it exists.
    #[must_use]
    pub fn resolve(&self, alias: &str) -> Option<Address> {
        self.inner
            .read()
            .expect("target directory lock poisoned")
            .aliases
            .get(alias)
            .cloned()
    }

    /// Removes and returns an address alias, if it exists.
    pub fn remove(&self, alias: &str) -> Option<Address> {
        let mut state = self.inner.write().expect("target directory lock poisoned");
        let removed = state.aliases.remove(alias);
        if let Some(address) = &removed {
            state.retained_bytes = state
                .retained_bytes
                .saturating_sub(target_entry_bytes(alias, address));
        }
        removed
    }

    /// Returns address aliases in deterministic lexical order.
    #[must_use]
    pub fn aliases(&self) -> Vec<Arc<str>> {
        self.inner
            .read()
            .expect("target directory lock poisoned")
            .aliases
            .keys()
            .cloned()
            .collect()
    }

    /// Returns the number of stored aliases.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .expect("target directory lock poisoned")
            .aliases
            .len()
    }

    /// Returns whether no aliases are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn target_entry_bytes(alias: &str, address: &Address) -> usize {
    let target_bytes = serde_json::to_vec(&address.target).map_or(usize::MAX, |value| value.len());
    let selection_bytes = match &address.bot {
        BotSelection::Current => 0,
        BotSelection::Exact(identity) => identity
            .platform
            .as_str()
            .len()
            .saturating_add(identity.bot.as_str().len()),
        BotSelection::Platform(platform) => platform.as_str().len(),
    };
    alias
        .len()
        .saturating_add(target_bytes)
        .saturating_add(selection_bytes)
        .saturating_add(128)
}
