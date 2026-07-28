use crate::{Address, BotDirectory, BotSelection, HandlerError, HandlerResult, Receipt, Reply};
use oxidebot_core::{
    source::message::Message, BotIdentity, FallbackPolicy, MessageTarget,
};

/// The ordinary immediate-send API used inside a handler.
///
/// Returning a `Message` remains the shortest path. `Messenger` is used when a
/// handler needs a receipt before it finishes, wants to quote the incoming
/// message, or needs to send to an explicit address. It keeps current and
/// cross-bot sends on the same capability-aware delivery pipeline.
#[derive(Clone)]
pub struct Messenger {
    reply: Reply,
    identity: BotIdentity,
    bots: Option<BotDirectory>,
}

impl Messenger {
    pub(crate) fn new(reply: Reply, identity: BotIdentity, bots: Option<BotDirectory>) -> Self {
        Self {
            reply,
            identity,
            bots,
        }
    }

    #[must_use]
    pub fn identity(&self) -> &BotIdentity {
        &self.identity
    }

    #[must_use]
    pub fn target(&self) -> &MessageTarget {
        self.reply.target()
    }

    /// Overrides capability fallback for subsequent sends.
    #[must_use]
    pub fn fallback(mut self, fallback: FallbackPolicy) -> Self {
        self.reply = self.reply.fallback(fallback);
        self
    }

    /// Sends to the current event's natural destination without quoting it.
    pub async fn send(&self, message: impl Into<Message>) -> HandlerResult<Receipt> {
        self.reply.send(message).await
    }

    /// Sends to the current event's natural destination and quotes the incoming
    /// message when the platform supports replies.
    pub async fn reply(&self, message: impl Into<Message>) -> HandlerResult<Receipt> {
        self.reply.reply(message).await
    }

    /// Binds this messenger to an explicit address. Selection is deterministic:
    /// an exact bot is used directly, a platform must have one connected bot,
    /// and `Current` retains the handler's current bot.
    pub fn to(&self, address: impl Into<Address>) -> HandlerResult<BoundMessenger> {
        let address = address.into();
        let reply = match &address.bot {
            BotSelection::Current => self.reply.retarget(address.target),
            BotSelection::Exact(identity) if identity == &self.identity => {
                self.reply.retarget(address.target)
            }
            BotSelection::Platform(platform) if platform == &self.identity.platform => {
                // Prefer the current bot for a platform selection made inside a
                // handler. This is deterministic and avoids surprising hops.
                self.reply.retarget(address.target)
            }
            selection => {
                let directory = self.bots.as_ref().ok_or_else(|| {
                    HandlerError::Api("the connected bot directory is not available".into())
                })?;
                let handle = directory.select(selection)?;
                let api = handle
                    .api()
                    .map_err(|error| HandlerError::Api(error.to_string()))?;
                self.reply.rebind(api, address.target)
            }
        };
        Ok(BoundMessenger { reply })
    }

    /// Convenience form of `messenger.to(address)?.send(message).await`.
    pub async fn send_to(
        &self,
        address: impl Into<Address>,
        message: impl Into<Message>,
    ) -> HandlerResult<Receipt> {
        self.to(address)?.send(message).await
    }
}

impl std::fmt::Debug for Messenger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Messenger")
            .field("identity", &self.identity)
            .field("target", &self.target())
            .field("connected_bots", &self.bots.as_ref().map(BotDirectory::len))
            .finish_non_exhaustive()
    }
}

/// A messenger already bound to one explicit target and selected bot.
#[derive(Clone, Debug)]
pub struct BoundMessenger {
    reply: Reply,
}

impl BoundMessenger {
    #[must_use]
    pub fn target(&self) -> &MessageTarget {
        self.reply.target()
    }

    #[must_use]
    pub fn fallback(mut self, fallback: FallbackPolicy) -> Self {
        self.reply = self.reply.fallback(fallback);
        self
    }

    pub async fn send(&self, message: impl Into<Message>) -> HandlerResult<Receipt> {
        self.reply.send(message).await
    }
}
