use crate::{BotHandle, HandlerError, HandlerResult};
use oxidebot_core::{
    content::MessageVisibility,
    conversation::MessageRef,
    interaction::{InteractionResponse, InteractionResponseHandle, InteractionVisibility},
    source::message::Message,
};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

const PENDING: u8 = 0;
const RESPONDING: u8 = 1;
const ACKNOWLEDGED: u8 = 2;
const DEFERRED: u8 = 3;
const EXPIRED: u8 = 4;

/// Observable lifecycle state of an answerable interaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionStatus {
    /// No initial response has claimed the interaction.
    Pending,
    /// One task is currently sending the initial response.
    Responding,
    /// The interaction has received its initial acknowledgement or response.
    Acknowledged,
    /// The interaction was deferred for a later response.
    Deferred,
    /// The platform acknowledgement deadline elapsed.
    Expired,
}

struct ResponderInner {
    bot: BotHandle,
    handle: InteractionResponseHandle,
    status: AtomicU8,
    settled: Arc<tokio::sync::Notify>,
}

/// A scheduler-backed interaction responder shared by the handler, automatic
/// replies, and deadline protection.
///
/// Clones refer to one atomic acknowledgement state. Only one initial
/// response or defer can win; subsequent messages become follow-ups when the
/// platform supports them.
#[derive(Clone)]
pub struct Responder(Arc<ResponderInner>);

impl Responder {
    pub(crate) fn new(bot: BotHandle, handle: InteractionResponseHandle) -> Self {
        let status = if handle.ack_required {
            PENDING
        } else {
            ACKNOWLEDGED
        };
        Self(Arc::new(ResponderInner {
            bot,
            handle,
            status: AtomicU8::new(status),
            settled: Arc::new(tokio::sync::Notify::new()),
        }))
    }

    /// Returns the immutable platform response handle.
    #[must_use]
    pub fn handle(&self) -> &InteractionResponseHandle {
        &self.0.handle
    }

    /// Returns the current shared acknowledgement state.
    #[must_use]
    pub fn status(&self) -> InteractionStatus {
        match self.0.status.load(Ordering::Acquire) {
            PENDING => InteractionStatus::Pending,
            RESPONDING => InteractionStatus::Responding,
            ACKNOWLEDGED => InteractionStatus::Acknowledged,
            DEFERRED => InteractionStatus::Deferred,
            _ => InteractionStatus::Expired,
        }
    }

    /// Returns whether no initial response has been claimed yet.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.status() == InteractionStatus::Pending
    }

    /// Sends an empty acknowledgement exactly once.
    pub async fn acknowledge(&self) -> HandlerResult<()> {
        self.initial(InteractionResponse::Acknowledge, ACKNOWLEDGED)
            .await
    }

    /// Defers the interaction exactly once, reserving later edits/follow-ups.
    pub async fn defer(&self) -> HandlerResult<()> {
        self.defer_with(InteractionVisibility::Public).await
    }

    /// Defers the interaction with explicit response visibility.
    pub async fn defer_with(&self, visibility: InteractionVisibility) -> HandlerResult<()> {
        self.claim_pending()?;
        match self
            .0
            .bot
            .defer_interaction(self.0.handle.clone(), visibility)
            .await
        {
            Ok(()) => {
                self.0.status.store(DEFERRED, Ordering::Release);
                self.0.settled.notify_one();
                Ok(())
            }
            Err(error) => {
                self.restore_after_failure();
                Err(HandlerError::from(error))
            }
        }
    }

    /// Sends an interaction message. If this responder was already
    /// acknowledged or deferred, the message is sent as a follow-up.
    pub async fn respond(&self, message: impl Into<Message>) -> HandlerResult<Vec<MessageRef>> {
        self.respond_with(message, InteractionVisibility::Public)
            .await
    }

    /// Sends an interaction message with explicit visibility.
    pub async fn respond_with(
        &self,
        message: impl Into<Message>,
        visibility: InteractionVisibility,
    ) -> HandlerResult<Vec<MessageRef>> {
        let mut message = message.into();
        message.options.visibility = match visibility {
            InteractionVisibility::Public => MessageVisibility::Public,
            InteractionVisibility::Ephemeral => MessageVisibility::Ephemeral,
        };
        match self.status() {
            InteractionStatus::Pending => {
                self.initial(
                    InteractionResponse::Message {
                        message,
                        visibility,
                    },
                    ACKNOWLEDGED,
                )
                .await?;
                Ok(Vec::new())
            }
            InteractionStatus::Acknowledged | InteractionStatus::Deferred => {
                self.follow_up(message).await
            }
            InteractionStatus::Responding => Err(HandlerError::Api(
                "the interaction response is already in progress".into(),
            )),
            InteractionStatus::Expired => Err(HandlerError::Api(
                "the interaction response deadline has expired".into(),
            )),
        }
    }

    /// Replaces the message associated with the interaction as its initial
    /// response, or edits the original response after acknowledgement.
    pub async fn update(&self, message: impl Into<Message>) -> HandlerResult<()> {
        let message = message.into();
        match self.status() {
            InteractionStatus::Pending => {
                self.initial(InteractionResponse::UpdateMessage { message }, ACKNOWLEDGED)
                    .await
            }
            InteractionStatus::Acknowledged | InteractionStatus::Deferred => {
                self.edit_original(message).await
            }
            InteractionStatus::Responding => Err(HandlerError::Api(
                "the interaction response is already in progress".into(),
            )),
            InteractionStatus::Expired => Err(HandlerError::Api(
                "the interaction response deadline has expired".into(),
            )),
        }
    }

    /// Edits the original response after acknowledgement or defer.
    pub async fn edit_original(&self, message: impl Into<Message>) -> HandlerResult<()> {
        self.require_acknowledged()?;
        self.0
            .bot
            .edit_interaction_response(self.0.handle.clone(), message.into())
            .await
            .map_err(HandlerError::from)
    }

    /// Sends one follow-up after acknowledgement or defer.
    pub async fn follow_up(&self, message: impl Into<Message>) -> HandlerResult<Vec<MessageRef>> {
        self.require_acknowledged()?;
        if !self.0.handle.followups_supported {
            return Err(HandlerError::Api(
                "the platform does not support interaction follow-up messages".into(),
            ));
        }
        self.0
            .bot
            .send_interaction_followup(self.0.handle.clone(), message.into())
            .await
            .map_err(HandlerError::from)
    }

    /// Returns validation errors to a modal/form interaction.
    pub async fn form_error(
        &self,
        errors: impl IntoIterator<Item = (String, String)>,
    ) -> HandlerResult<()> {
        self.initial(
            InteractionResponse::ValidationErrors(errors.into_iter().collect()),
            ACKNOWLEDGED,
        )
        .await
    }

    pub(crate) fn start_auto_defer(&self) {
        if !self.is_pending() {
            return;
        }
        let Some(deadline) = self.0.handle.deadline else {
            return;
        };
        let weak = Arc::downgrade(&self.0);
        let settled = Arc::clone(&self.0.settled);
        tokio::spawn(async move {
            let remaining = deadline.signed_duration_since(chrono::Utc::now());
            let delay = remaining
                .to_std()
                .unwrap_or_default()
                .saturating_sub(std::time::Duration::from_secs(1));
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = settled.notified() => return,
            }
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let responder = Responder(inner);
            if responder.is_pending() {
                if let Err(error) = responder.defer().await {
                    tracing::warn!(%error, "could not automatically defer interaction before its deadline");
                }
            }
        });
    }

    async fn initial(&self, response: InteractionResponse, final_status: u8) -> HandlerResult<()> {
        self.claim_pending()?;
        match self
            .0
            .bot
            .answer_interaction(self.0.handle.clone(), response)
            .await
        {
            Ok(()) => {
                self.0.status.store(final_status, Ordering::Release);
                self.0.settled.notify_one();
                Ok(())
            }
            Err(error) => {
                self.restore_after_failure();
                Err(HandlerError::from(error))
            }
        }
    }

    fn claim_pending(&self) -> HandlerResult<()> {
        if self.deadline_expired() {
            self.0.status.store(EXPIRED, Ordering::Release);
            self.0.settled.notify_one();
            return Err(HandlerError::Api(
                "the interaction response deadline has expired".into(),
            ));
        }
        self.0
            .status
            .compare_exchange(PENDING, RESPONDING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|status| {
                HandlerError::Api(
                    match status {
                        RESPONDING => "the interaction response is already in progress",
                        ACKNOWLEDGED | DEFERRED => "the interaction was already acknowledged",
                        _ => "the interaction response deadline has expired",
                    }
                    .into(),
                )
            })
    }

    fn require_acknowledged(&self) -> HandlerResult<()> {
        match self.status() {
            InteractionStatus::Acknowledged | InteractionStatus::Deferred => Ok(()),
            InteractionStatus::Pending => Err(HandlerError::Api(
                "the interaction must be acknowledged or deferred first".into(),
            )),
            InteractionStatus::Responding => Err(HandlerError::Api(
                "the interaction response is already in progress".into(),
            )),
            InteractionStatus::Expired => Err(HandlerError::Api(
                "the interaction response deadline has expired".into(),
            )),
        }
    }

    fn deadline_expired(&self) -> bool {
        self.0
            .handle
            .deadline
            .is_some_and(|deadline| chrono::Utc::now() >= deadline)
    }

    fn restore_after_failure(&self) {
        let status = if self.deadline_expired() {
            EXPIRED
        } else {
            PENDING
        };
        self.0.status.store(status, Ordering::Release);
    }
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Responder")
            .field("handle", &self.0.handle)
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}
