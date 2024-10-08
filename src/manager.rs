use std::{future::Future, pin::Pin};

use crate::{
    bot::{add_bots, BotObject},
    filter::{FilterObject, FilterPool},
    handler::{EventHandlerPool, Handler},
    matcher::Matcher,
};
use tokio::sync::broadcast;

pub struct BroadcastSender(broadcast::Sender<Matcher>);

impl BroadcastSender {
    pub(crate) fn new(sender: broadcast::Sender<Matcher>) -> Self {
        BroadcastSender(sender)
    }

    pub(crate) fn clone_sender(&self) -> broadcast::Sender<Matcher> {
        self.0.clone()
    }

    // handler maker can only use methods below

    pub fn clone(&self) -> Self {
        BroadcastSender(self.0.clone())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Matcher> {
        self.0.subscribe()
    }
}

/// OxideBotManager is the main struct and the entry of OxideBot
/// Use it to build your bot framework and run it.
pub struct OxideBotManager {
    handler_pool: EventHandlerPool,
    filter_pool: FilterPool,
    broadcast_sender: BroadcastSender,
    broadcast_receiver: broadcast::Receiver<Matcher>,
}

impl OxideBotManager {
    /// Create a new OxideBotManager
    pub fn new() -> Self {
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(100);
        OxideBotManager {
            handler_pool: EventHandlerPool::new(),
            filter_pool: FilterPool::new(),
            broadcast_sender: BroadcastSender::new(broadcast_sender),
            broadcast_receiver,
        }
    }
    /// Build a OxideBotManager with bots, handlers and filters
    pub async fn build(
        bots: Vec<BotObject>,
        handlers: Vec<Handler>,
        filters: Vec<FilterObject>,
    ) -> Self {
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(100);
        add_bots(bots, broadcast_sender.clone()).await;
        OxideBotManager {
            handler_pool: EventHandlerPool::build(handlers),
            filter_pool: FilterPool::build(filters),
            broadcast_sender: BroadcastSender::new(broadcast_sender),
            broadcast_receiver,
        }
    }
    /// Add a bot to the OxideBotManager
    pub async fn bot(self, bot: BotObject) -> Self {
        add_bots(vec![bot.into()], self.broadcast_sender.clone_sender()).await;
        self
    }
    /// Add a handler to the OxideBotManager
    pub fn handler<H: Into<Handler>>(mut self, handler: H) -> Self {
        self.handler_pool.add_handler(handler.into());
        self
    }
    /// Add a handler to the OxideBotManager with a handler creator
    pub async fn wait_handler(
        self,
        handler_creator: impl Fn(BroadcastSender) -> Pin<Box<dyn Future<Output = Handler>>>,
    ) -> Self {
        let handler = handler_creator(self.broadcast_sender.clone()).await;
        self.handler(handler)
    }
    /// Add a filter to the OxideBotManager
    pub fn filter<F: Into<FilterObject>>(mut self, filter: F) -> Self {
        self.filter_pool.add_filter(filter.into());
        self
    }
    /// Run the OxideBotManager, this function will block the current thread
    pub async fn run_block(mut self) -> ! {
        loop {
            if let Ok(matcher) = self.broadcast_receiver.recv().await {
                if self.filter_pool.filter(matcher.clone()).await {
                    self.handler_pool.handle(matcher);
                }
            }
        }
    }
}
