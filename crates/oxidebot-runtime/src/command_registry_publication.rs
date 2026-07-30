//! Native command-definition publication synchronization for a registry.

use super::*;

#[derive(Clone, Debug)]
pub(in crate::command_registry) struct CommandPublicationState {
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

impl CommandRegistry {
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
}
