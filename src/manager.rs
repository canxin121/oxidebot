use tokio::sync::mpsc;

use crate::{
    bot::{add_bots, BotObject},
    filter::{FilterObject, FilterPool},
    handler::{EventHandlerPool, Handler},
    matcher::Matcher,
};

pub struct OxideBotManager {
    handler_pool: EventHandlerPool,
    filter_pool: FilterPool,
    receiver: mpsc::Receiver<Matcher>,
    sender: mpsc::Sender<Matcher>,
}

impl OxideBotManager {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel(100);
        OxideBotManager {
            handler_pool: EventHandlerPool::new(),
            filter_pool: FilterPool::new(),
            receiver,
            sender,
        }
    }

    pub async fn bot<B: Into<BotObject>>(self, bot: B) -> Self {
        add_bots(vec![bot.into()], self.sender.clone()).await;
        self
    }

    pub fn handler<H: Into<Handler>>(mut self, handler: H) -> Self {
        self.handler_pool.add_handler(handler.into());
        self
    }

    pub fn filter<F: Into<FilterObject>>(mut self, filter: F) -> Self {
        self.filter_pool.add_filter(filter.into());
        self
    }

    pub async fn build(
        bots: Vec<BotObject>,
        handlers: Vec<Handler>,
        filters: Vec<FilterObject>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        add_bots(bots, sender.clone()).await;
        OxideBotManager {
            handler_pool: EventHandlerPool::build(handlers),
            filter_pool: FilterPool::build(filters),
            receiver,
            sender,
        }
    }

    pub async fn run_block(mut self) {
        while let Some(matcher) = self.receiver.recv().await {
            if self.filter_pool.filter(matcher.clone()).await {
                self.handler_pool.handle(matcher);
            }
        }
    }
}
