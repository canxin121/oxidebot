use crate::{
    bot::{add_bots, BotObject},
    filter::{FilterObject, FilterPool},
    handler::{EventHandlerPool, Handler},
    matcher::Matcher,
};
use tokio::sync::broadcast;

pub struct OxideBotManager {
    handler_pool: EventHandlerPool,
    filter_pool: FilterPool,
    broadcast_sender: broadcast::Sender<Matcher>,
    broadcast_receiver: broadcast::Receiver<Matcher>,
}

impl OxideBotManager {
    pub fn new() -> Self {
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(100);
        OxideBotManager {
            handler_pool: EventHandlerPool::new(),
            filter_pool: FilterPool::new(),
            broadcast_sender,
            broadcast_receiver,
        }
    }

    pub async fn bot(self, bot: BotObject) -> Self {
        add_bots(vec![bot.into()], self.broadcast_sender.clone()).await;
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
        let (broadcast_sender, broadcast_receiver) = broadcast::channel(100);
        add_bots(bots, broadcast_sender.clone()).await;
        OxideBotManager {
            handler_pool: EventHandlerPool::build(handlers),
            filter_pool: FilterPool::build(filters),
            broadcast_sender,
            broadcast_receiver,
        }
    }

    pub async fn run_block(mut self) {
        loop {
            match self.broadcast_receiver.recv().await {
                Ok(matcher) => {
                    if self.filter_pool.filter(matcher.clone()).await {
                        self.handler_pool.handle(matcher);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // 可以根据需求处理消息滞后的情况
                }
            }
        }
    }
}


#[cfg(test)]
mod test_manager {
    use super::OxideBotManager;
    use crate::{handler::Handler, matcher::Matcher, EventHandlerTrait};
    use anyhow::Result;

    pub struct MyHandler;
    impl MyHandler {
        pub fn new() -> Handler {
            Handler {
                event_handler: Some(Box::new(MyHandler)),
                active_handler: None,
            }
        }
    }

    impl EventHandlerTrait for MyHandler {
        #[must_use]
        #[allow(
            elided_named_lifetimes,
            clippy::type_complexity,
            clippy::type_repetition_in_bounds
        )]
        fn handle<'life0, 'async_trait>(
            &'life0 self,
            matcher: Matcher,
        ) -> ::core::pin::Pin<
            Box<
                dyn ::core::future::Future<Output = Result<()>>
                    + ::core::marker::Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            Self: 'async_trait,
        {
            todo!()
        }
    }

    #[tokio::test]
    async fn test_main() {
        let manager = OxideBotManager::new().handler(
            MyHandler::new(),
        );

        tokio::spawn(async move {
            manager.run_block().await;
        });
    }
}
