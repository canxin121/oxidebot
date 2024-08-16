use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::task::JoinHandle;

use crate::matcher::Matcher;

#[async_trait]
pub trait ActiveHandlerTrait: Send + Sync {
    async fn run_forever(&self) -> Result<()>;
}

#[async_trait]
pub trait EventHandlerTrait: Send + Sync {
    async fn handle(&self, matcher: Matcher) -> Result<()>;
}

pub type EventHandlerObject = Box<dyn EventHandlerTrait>;

pub type ActiveHandlerObject = Box<dyn ActiveHandlerTrait>;

#[derive(Default)]
pub struct Handler {
    pub event_handler: Option<EventHandlerObject>,
    pub active_handler: Option<ActiveHandlerObject>,
}

pub struct EventHandlerPool {
    event_handlers: Vec<Arc<EventHandlerObject>>,
    active_handler_joinhandsles: Vec<JoinHandle<()>>,
}

impl EventHandlerPool {
    pub fn new() -> Self {
        EventHandlerPool {
            event_handlers: Vec::new(),
            active_handler_joinhandsles: Vec::new(),
        }
    }

    pub fn build(handlers: Vec<Handler>) -> Self {
        let mut pool = EventHandlerPool {
            event_handlers: Vec::new(),
            active_handler_joinhandsles: Vec::new(),
        };
        for handler in handlers {
            pool.add_handler(handler);
        }
        pool
    }

    pub fn add_handler(&mut self, handler: Handler) {
        if let Some(event_handler) = handler.event_handler {
            self.event_handlers.push(event_handler.into());
        }
        if let Some(active_handler) = handler.active_handler {
            let active_handler = Arc::new(active_handler);
            let join_handle = tokio::spawn(async move {
                if let Err(e) = active_handler.run_forever().await {
                    tracing::error!("Active handler error: {:?}", e);
                }
            });
            self.active_handler_joinhandsles.push(join_handle);
        }
    }

    pub fn handle(&self, matcher: Matcher) {
        for handler in &self.event_handlers {
            let handler = Arc::clone(handler);
            let matcher = matcher.clone();
            tokio::spawn(async move {
                if let Err(e) = handler.handle(matcher).await {
                    tracing::error!("Event handler error: {:?}", e);
                }
            });
        }
    }
}
