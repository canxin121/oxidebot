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

pub struct OxideBotManager {
    handler_pool: EventHandlerPool,
    filter_pool: FilterPool,
    broadcast_sender: BroadcastSender,
    broadcast_receiver: broadcast::Receiver<Matcher>,
}

impl OxideBotManager {
    pub fn new() -> Self {
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(100);
        OxideBotManager {
            handler_pool: EventHandlerPool::new(),
            filter_pool: FilterPool::new(),
            broadcast_sender: BroadcastSender::new(broadcast_sender),
            broadcast_receiver,
        }
    }

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

    pub async fn bot(self, bot: BotObject) -> Self {
        add_bots(vec![bot.into()], self.broadcast_sender.clone_sender()).await;
        self
    }

    pub fn handler<H: Into<Handler>>(mut self, handler: H) -> Self {
        self.handler_pool.add_handler(handler.into());
        self
    }

    pub fn wait_handler<H>(self, handler_creator: impl FnOnce(BroadcastSender) -> H) -> Self
    where
        H: Into<Handler>,
    {
        let handler = handler_creator(self.broadcast_sender.clone());
        self.handler(handler)
    }

    pub fn filter<F: Into<FilterObject>>(mut self, filter: F) -> Self {
        self.filter_pool.add_filter(filter.into());
        self
    }

    pub async fn run_block(mut self) {
        loop {
            if let Ok(matcher) = self.broadcast_receiver.recv().await {
                if self.filter_pool.filter(matcher.clone()).await {
                    self.handler_pool.handle(matcher);
                }
            }
        }
    }
}
