//! Bounded mutable command state and native publication synchronization.
//!
//! This registry owns enablement and runtime shortcuts after a command tree has
//! been built. It intentionally remains distinct from the immutable command
//! schema and from authoring middleware.

use crate::{
    BotDirectory, Command, CommandId, Context, Extract, ExtractError, HandlerError, HandlerResult,
    Shortcut, MAX_REGISTRY_SHORTCUTS, MAX_SHORTCUT_SCAN_PER_MESSAGE,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

/// Bounded, mutable state for runtime command enablement and shortcuts.
#[derive(Clone, Debug)]
pub struct CommandRegistry {
    inner: Arc<RwLock<CommandRegistryState>>,
    publication: Arc<RwLock<Option<CommandPublicationState>>>,
    publication_gate: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone, Debug)]
struct CommandPublicationState {
    bots: BotDirectory,
    catalog: crate::CommandCatalog,
    published_revision: Option<u64>,
    pending_revision: Option<u64>,
    last_error: Option<Arc<str>>,
}

/// Snapshot of local-to-platform command-definition synchronization.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandPublicationStatus {
    /// Current local command-registry revision requested for publication.
    pub desired_revision: u64,
    /// Most recently published revision, if synchronization has succeeded.
    pub published_revision: Option<u64>,
    /// Revision currently waiting to be published, if any.
    pub pending_revision: Option<u64>,
    /// Last publication failure, if one occurred.
    pub last_error: Option<Arc<str>>,
    /// Whether this registry is attached to connected bots for publication.
    pub attached: bool,
}

#[derive(Debug)]
struct CommandRegistryState {
    capacity: usize,
    max_bytes: usize,
    retained_bytes: usize,
    shortcut_count: usize,
    commands: HashMap<CommandId, RuntimeCommandState>,
    order: VecDeque<CommandId>,
    shortcut_order: VecDeque<CommandId>,
    revision: u64,
    dynamic_shortcuts: bool,
    broad_command_matching: bool,
}

/// Current runtime state for a registered command.
#[derive(Clone, Debug)]
pub struct RuntimeCommandState {
    /// Canonical command name.
    pub name: Arc<str>,
    /// Whether dispatch currently permits this command.
    pub enabled: bool,
    schema_fingerprint: u64,
    /// Shortcuts compiled with the command definition. They are immutable at runtime.
    pub static_shortcuts: Vec<Shortcut>,
    /// Bounded shortcuts added through the runtime command registry.
    pub shortcuts: Vec<Shortcut>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::bounded(16_384)
    }
}

impl CommandRegistry {
    /// Creates a registry bounded by the number of commands it retains.
    #[must_use]
    pub fn bounded(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let max_bytes = capacity.saturating_mul(32 * 1024).min(512 * 1024 * 1024);
        Self::bounded_bytes(capacity, max_bytes)
    }

    /// Creates a registry bounded by command count, total shortcut count, and
    /// retained command/shortcut bytes.
    #[must_use]
    pub fn bounded_bytes(capacity: usize, max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(CommandRegistryState {
                capacity: capacity.max(1),
                max_bytes: max_bytes.max(1),
                retained_bytes: 0,
                shortcut_count: 0,
                commands: HashMap::new(),
                order: VecDeque::new(),
                shortcut_order: VecDeque::new(),
                revision: 0,
                dynamic_shortcuts: false,
                broad_command_matching: false,
            })),
            publication: Arc::new(RwLock::new(None)),
            publication_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Registers static metadata for a command and initializes its runtime state.
    pub fn register(&self, command: &Command) -> Result<(), HandlerError> {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        for shortcut in command.shortcuts() {
            shortcut.validate()?;
        }
        let schema_fingerprint = command.structural_fingerprint();
        if let Some(existing) = state.commands.get(&command.id()) {
            if existing.name.as_ref() != command.name()
                || existing.schema_fingerprint != schema_fingerprint
            {
                return Err(HandlerError::internal(format!(
                    "command ID collision or incompatible grammar between `{}` and `{}`",
                    existing.name,
                    command.name(),
                )));
            }
            let additions = command
                .shortcuts()
                .iter()
                .filter(|shortcut| {
                    !existing.static_shortcuts.iter().any(|current| {
                        current.is_regex() == shortcut.is_regex()
                            && current.pattern_text() == shortcut.pattern_text()
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            for shortcut in &additions {
                if state.commands.iter().any(|(registered_id, registered)| {
                    *registered_id != command.id()
                        && registered
                            .static_shortcuts
                            .iter()
                            .chain(registered.shortcuts.iter())
                            .any(|current| {
                                current.is_regex() == shortcut.is_regex()
                                    && current.pattern_text() == shortcut.pattern_text()
                            })
                }) {
                    return Err(HandlerError::user(
                        "a static shortcut conflicts with an already registered command",
                    ));
                }
            }
            if !additions.is_empty() {
                let additions_len = additions.len();
                if existing
                    .static_shortcuts
                    .len()
                    .saturating_add(additions.len())
                    > 256
                {
                    return Err(HandlerError::internal("shortcut limit exceeded"));
                }
                let added_bytes = additions
                    .iter()
                    .map(Shortcut::retained_bytes)
                    .sum::<usize>();
                if state.shortcut_count.saturating_add(additions.len()) > MAX_REGISTRY_SHORTCUTS {
                    return Err(HandlerError::internal("global shortcut capacity exceeded"));
                }
                if state.retained_bytes.saturating_add(added_bytes) > state.max_bytes {
                    return Err(HandlerError::internal(
                        "command registry byte capacity exceeded",
                    ));
                }
                let had_shortcuts =
                    !existing.static_shortcuts.is_empty() || !existing.shortcuts.is_empty();
                state
                    .commands
                    .get_mut(&command.id())
                    .expect("command existence checked above")
                    .static_shortcuts
                    .extend(additions);
                state.shortcut_count = state.shortcut_count.saturating_add(additions_len);
                state.retained_bytes = state.retained_bytes.saturating_add(added_bytes);
                if !had_shortcuts {
                    state.shortcut_order.push_back(command.id());
                }
                state.revision = state.revision.wrapping_add(1);
            }
            return Ok(());
        }
        if state.commands.len() >= state.capacity {
            return Err(HandlerError::internal("command registry capacity exceeded"));
        }
        if command.shortcuts().len() > 256 {
            return Err(HandlerError::internal("shortcut limit exceeded"));
        }
        if state
            .shortcut_count
            .saturating_add(command.shortcuts().len())
            > MAX_REGISTRY_SHORTCUTS
        {
            return Err(HandlerError::internal("global shortcut capacity exceeded"));
        }
        let added_bytes = command
            .name()
            .len()
            .saturating_add(
                command
                    .shortcuts()
                    .iter()
                    .map(Shortcut::retained_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(256);
        if state.retained_bytes.saturating_add(added_bytes) > state.max_bytes {
            return Err(HandlerError::internal(
                "command registry byte capacity exceeded",
            ));
        }
        for shortcut in command.shortcuts() {
            if state.commands.values().any(|registered| {
                registered
                    .static_shortcuts
                    .iter()
                    .chain(registered.shortcuts.iter())
                    .any(|existing| {
                        existing.is_regex() == shortcut.is_regex()
                            && existing.pattern_text() == shortcut.pattern_text()
                    })
            }) {
                return Err(HandlerError::user(
                    "a static shortcut conflicts with an already registered command",
                ));
            }
        }
        state.order.push_back(command.id());
        if !command.shortcuts().is_empty() {
            state.shortcut_order.push_back(command.id());
        }
        state.commands.insert(
            command.id(),
            RuntimeCommandState {
                name: Arc::from(command.name()),
                enabled: true,
                schema_fingerprint,
                static_shortcuts: command.shortcuts().to_vec(),
                shortcuts: Vec::new(),
            },
        );
        state.shortcut_count = state
            .shortcut_count
            .saturating_add(command.shortcuts().len());
        state.retained_bytes = state.retained_bytes.saturating_add(added_bytes);
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }

    /// Finds a registered command by canonical name, case-insensitively.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<CommandId> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .iter()
            .find_map(|(id, command)| command.name.eq_ignore_ascii_case(name).then_some(*id))
    }

    /// Returns whether a registered command is enabled; unknown IDs are treated as enabled.
    #[must_use]
    pub fn is_enabled(&self, id: CommandId) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .is_none_or(|command| command.enabled)
    }

    /// Enables a command and returns whether its state changed.
    pub fn enable(&self, id: CommandId) -> bool {
        self.set_enabled(id, true)
    }

    /// Disables a command and returns whether its state changed.
    pub fn disable(&self, id: CommandId) -> bool {
        self.set_enabled(id, false)
    }

    fn set_enabled(&self, id: CommandId, enabled: bool) -> bool {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let Some(command) = state.commands.get_mut(&id) else {
            return false;
        };
        if command.enabled == enabled {
            return false;
        }
        command.enabled = enabled;
        state.revision = state.revision.wrapping_add(1);
        true
    }

    /// Adds a bounded runtime shortcut to a registered command.
    pub fn add_shortcut(&self, id: CommandId, shortcut: Shortcut) -> Result<(), HandlerError> {
        shortcut.validate()?;
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let command_state = state
            .commands
            .get(&id)
            .ok_or_else(|| HandlerError::internal("unknown command id"))?;
        let existing_count = command_state.shortcuts.len();
        let had_any_shortcuts =
            !command_state.static_shortcuts.is_empty() || !command_state.shortcuts.is_empty();
        if existing_count >= 256 {
            return Err(HandlerError::internal("shortcut limit exceeded"));
        }
        if state.shortcut_count >= MAX_REGISTRY_SHORTCUTS {
            return Err(HandlerError::internal("global shortcut capacity exceeded"));
        }
        let shortcut_bytes = shortcut.retained_bytes();
        if state.retained_bytes.saturating_add(shortcut_bytes) > state.max_bytes {
            return Err(HandlerError::internal(
                "command registry byte capacity exceeded",
            ));
        }
        let duplicate = state.commands.iter().any(|(_, command)| {
            command
                .static_shortcuts
                .iter()
                .chain(command.shortcuts.iter())
                .any(|existing| {
                    existing.is_regex() == shortcut.is_regex()
                        && existing.pattern_text() == shortcut.pattern_text()
                })
        });
        if duplicate {
            return Err(HandlerError::user(
                "the shortcut conflicts with an already registered command",
            ));
        }
        if !had_any_shortcuts {
            state.shortcut_order.push_back(id);
        }
        state
            .commands
            .get_mut(&id)
            .expect("command existence checked above")
            .shortcuts
            .push(shortcut);
        state.shortcut_count = state.shortcut_count.saturating_add(1);
        state.retained_bytes = state.retained_bytes.saturating_add(shortcut_bytes);
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }

    /// Removes runtime shortcuts matching `pattern`, returning whether any were removed.
    pub fn remove_shortcut(&self, id: CommandId, pattern: &str) -> Result<bool, HandlerError> {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let (changed, removed, removed_bytes, has_any_shortcuts) = {
            let command = state
                .commands
                .get_mut(&id)
                .ok_or_else(|| HandlerError::internal("unknown command id"))?;
            let before = command.shortcuts.len();
            let removed_bytes = command
                .shortcuts
                .iter()
                .filter(|shortcut| shortcut.pattern_text() == pattern)
                .map(Shortcut::retained_bytes)
                .sum::<usize>();
            command
                .shortcuts
                .retain(|shortcut| shortcut.pattern_text() != pattern);
            let removed = before.saturating_sub(command.shortcuts.len());
            (
                removed > 0,
                removed,
                removed_bytes,
                !command.static_shortcuts.is_empty() || !command.shortcuts.is_empty(),
            )
        };
        if changed {
            state.shortcut_count = state.shortcut_count.saturating_sub(removed);
            state.retained_bytes = state.retained_bytes.saturating_sub(removed_bytes);
            if !has_any_shortcuts {
                state.shortcut_order.retain(|command_id| *command_id != id);
            }
            state.revision = state.revision.wrapping_add(1);
        }
        Ok(changed)
    }

    /// Returns up to `limit` enabled command IDs whose shortcuts match `input`.
    #[must_use]
    pub fn matching_shortcut_commands(&self, input: &str, limit: usize) -> Vec<CommandId> {
        let candidates = {
            let state = self.inner.read().expect("command registry lock poisoned");
            let mut remaining = MAX_SHORTCUT_SCAN_PER_MESSAGE;
            let mut candidates = Vec::new();
            for id in &state.shortcut_order {
                if remaining == 0 {
                    break;
                }
                let Some(command) = state.commands.get(id) else {
                    continue;
                };
                if !command.enabled {
                    continue;
                }
                let shortcuts = command
                    .static_shortcuts
                    .iter()
                    .chain(command.shortcuts.iter())
                    .take(remaining)
                    .cloned()
                    .collect::<Vec<_>>();
                remaining = remaining.saturating_sub(shortcuts.len());
                candidates.push((*id, shortcuts));
            }
            candidates
        };
        let mut matches = Vec::new();
        for (id, shortcuts) in candidates {
            if shortcuts
                .iter()
                .any(|shortcut| shortcut.rewrite(input).is_some())
            {
                matches.push(id);
                if matches.len() >= limit {
                    break;
                }
            }
        }
        matches
    }

    /// Removes all mutable shortcuts for a command and returns the number removed.
    pub fn clear_shortcuts(&self, id: CommandId) -> usize {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        let Some(command) = state.commands.get_mut(&id) else {
            return 0;
        };
        let removed = command.shortcuts.len();
        let removed_bytes = command
            .shortcuts
            .iter()
            .map(Shortcut::retained_bytes)
            .sum::<usize>();
        command.shortcuts.clear();
        let has_static_shortcuts = !command.static_shortcuts.is_empty();
        if removed > 0 {
            state.shortcut_count = state.shortcut_count.saturating_sub(removed);
            state.retained_bytes = state.retained_bytes.saturating_sub(removed_bytes);
            if !has_static_shortcuts {
                state.shortcut_order.retain(|command_id| *command_id != id);
            }
            state.revision = state.revision.wrapping_add(1);
        }
        removed
    }

    /// Returns static and mutable shortcuts for a registered command.
    #[must_use]
    pub fn shortcuts(&self, id: CommandId) -> Vec<Shortcut> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .map_or_else(Vec::new, |command| {
                command
                    .static_shortcuts
                    .iter()
                    .chain(command.shortcuts.iter())
                    .cloned()
                    .collect()
            })
    }

    #[must_use]
    pub(crate) fn runtime_shortcuts_for(&self, id: CommandId) -> Vec<Shortcut> {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .commands
            .get(&id)
            .map_or_else(Vec::new, |command| command.shortcuts.clone())
    }

    pub(crate) fn configure_matching(&self, dynamic_shortcuts: bool, broad_command_matching: bool) {
        let mut state = self.inner.write().expect("command registry lock poisoned");
        state.dynamic_shortcuts = dynamic_shortcuts;
        state.broad_command_matching = broad_command_matching;
    }

    #[must_use]
    pub(crate) fn dynamic_shortcuts_enabled(&self) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .dynamic_shortcuts
    }

    #[must_use]
    pub(crate) fn broad_command_matching(&self) -> bool {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .broad_command_matching
    }

    /// Returns the monotonically changing local command-registry revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.inner
            .read()
            .expect("command registry lock poisoned")
            .revision
    }

    pub(crate) fn attach_publication(&self, bots: BotDirectory, catalog: crate::CommandCatalog) {
        let revision = self.revision();
        *self
            .publication
            .write()
            .expect("command publication lock poisoned") = Some(CommandPublicationState {
            bots,
            catalog,
            published_revision: None,
            pending_revision: Some(revision),
            last_error: None,
        });
    }

    pub(crate) fn detach_publication(&self) {
        *self
            .publication
            .write()
            .expect("command publication lock poisoned") = None;
    }

    /// Synchronizes changed command definitions with every attached adapter.
    pub async fn refresh_publication(&self) -> HandlerResult<()> {
        let _gate = self.publication_gate.lock().await;
        let desired_revision = self.revision();
        {
            let mut publication = self
                .publication
                .write()
                .expect("command publication lock poisoned");
            if let Some(publication) = publication.as_mut() {
                if publication.published_revision != Some(desired_revision) {
                    publication.pending_revision = Some(desired_revision);
                }
            }
        }
        let publication = self
            .publication
            .read()
            .expect("command publication lock poisoned")
            .clone();
        let Some(publication) = publication else {
            return Ok(());
        };
        if publication.pending_revision.is_none()
            && publication.published_revision == Some(desired_revision)
        {
            return Ok(());
        }
        match crate::app::publish_command_definitions(&publication.bots, &publication.catalog, self)
            .await
        {
            Ok(()) => {
                let current_revision = self.revision();
                let mut state = self
                    .publication
                    .write()
                    .expect("command publication lock poisoned");
                if let Some(state) = state.as_mut() {
                    state.published_revision = Some(desired_revision);
                    state.last_error = None;
                    state.pending_revision =
                        (current_revision != desired_revision).then_some(current_revision);
                }
                Ok(())
            }
            Err(error) => {
                let message: Arc<str> = Arc::from(error.to_string());
                let mut state = self
                    .publication
                    .write()
                    .expect("command publication lock poisoned");
                if let Some(state) = state.as_mut() {
                    state.pending_revision = Some(self.revision());
                    state.last_error = Some(Arc::clone(&message));
                }
                Err(HandlerError::Api(message.to_string()))
            }
        }
    }

    /// Enables a command and synchronizes changes with attached adapters.
    pub async fn enable_and_publish(&self, id: CommandId) -> HandlerResult<bool> {
        let changed = self.enable(id);
        if changed || self.publication_status().pending_revision.is_some() {
            self.refresh_publication().await?;
        }
        Ok(changed)
    }

    /// Disables a command and synchronizes changes with attached adapters.
    pub async fn disable_and_publish(&self, id: CommandId) -> HandlerResult<bool> {
        let changed = self.disable(id);
        if changed || self.publication_status().pending_revision.is_some() {
            self.refresh_publication().await?;
        }
        Ok(changed)
    }

    /// Returns whether platform command definitions have converged to the
    /// registry's current revision and retains the last retryable failure.
    #[must_use]
    pub fn publication_status(&self) -> CommandPublicationStatus {
        let desired_revision = self.revision();
        let publication = self
            .publication
            .read()
            .expect("command publication lock poisoned");
        publication.as_ref().map_or(
            CommandPublicationStatus {
                desired_revision,
                ..CommandPublicationStatus::default()
            },
            |state| CommandPublicationStatus {
                desired_revision,
                published_revision: state.published_revision,
                pending_revision: state.pending_revision.or_else(|| {
                    (state.published_revision != Some(desired_revision)).then_some(desired_revision)
                }),
                last_error: state.last_error.clone(),
                attached: true,
            },
        )
    }

    /// Returns registered command states in deterministic registration order.
    pub fn snapshot(&self) -> Vec<(CommandId, RuntimeCommandState)> {
        let state = self.inner.read().expect("command registry lock poisoned");
        state
            .order
            .iter()
            .filter_map(|id| state.commands.get(id).cloned().map(|value| (*id, value)))
            .collect()
    }
}

impl<S> Extract<S> for CommandRegistry
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(context.authoring().registry.clone())
    }
}
