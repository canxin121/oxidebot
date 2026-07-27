use crate::{
    adapter::InterestPlan, session::SessionRegistry, BotHandle, ErasedHandler, Filter,
    MetricsHandle,
};
use futures_util::FutureExt;
use oxidebot_core::{EventEnvelope, MessageTarget};
use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub(crate) struct CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    handlers: Vec<Arc<dyn ErasedHandler<S>>>,
    filters: Vec<Arc<dyn Filter<S>>>,
    state: Arc<S>,
    sessions: SessionRegistry,
    cancellation: CancellationToken,
    handler_timeout: Option<Duration>,
    metrics: MetricsHandle,
    interest: InterestPlan,
}

impl<S> CompiledRouter<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) fn compile(
        handlers: Vec<Arc<dyn ErasedHandler<S>>>,
        filters: Vec<Arc<dyn Filter<S>>>,
        state: Arc<S>,
        sessions: SessionRegistry,
        cancellation: CancellationToken,
        handler_timeout: Option<Duration>,
        metrics: MetricsHandle,
    ) -> Self {
        let mut interest = InterestPlan::default();
        for handler in &handlers {
            handler.add_interest(&mut interest);
        }
        interest.set_session_interest(sessions.interest());
        Self {
            handlers,
            filters,
            state,
            sessions,
            cancellation,
            handler_timeout,
            metrics,
            interest,
        }
    }
    pub(crate) fn interest(&self) -> InterestPlan {
        self.interest.clone()
    }
    pub(crate) async fn dispatch(&self, event: Arc<EventEnvelope>, bot: BotHandle) {
        if self
            .filters
            .iter()
            .any(|filter| !filter.accepts(&event, &self.state))
        {
            return;
        }
        for handler in &self.handlers {
            if !handler.matches(&event.index) {
                continue;
            }
            let future = handler.call(
                Arc::clone(&event),
                Arc::clone(&self.state),
                bot.clone(),
                self.sessions.clone(),
                self.cancellation.child_token(),
            );
            let result = if let Some(timeout) = self.handler_timeout {
                match tokio::time::timeout(timeout, AssertUnwindSafe(future).catch_unwind()).await {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::warn!(event_id = %event.id, "handler timed out");
                        continue;
                    }
                }
            } else {
                AssertUnwindSafe(future).catch_unwind().await
            };
            let outcome = match result {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    tracing::warn!(event_id = %event.id, %error, "handler failed");
                    continue;
                }
                Err(_) => {
                    self.metrics.handler_panic();
                    tracing::error!(event_id = %event.id, "handler panicked");
                    continue;
                }
            };
            if let Some(conversation) = event.index.conversation.clone() {
                let target = MessageTarget::new(conversation);
                for reply in outcome.replies {
                    if let Err(error) = bot.enqueue_send(target.clone(), reply).await {
                        tracing::warn!(event_id = %event.id, %error, "could not enqueue handler reply");
                    }
                }
            }
            if outcome.stop {
                break;
            }
        }
    }
}
