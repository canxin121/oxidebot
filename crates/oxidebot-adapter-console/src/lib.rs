//! A real, runnable stdin/stdout adapter for local OxideBot applications.
//!
//! Each input line is a direct message from one configurable console user. Bot
//! sends are rendered to stdout. The adapter is intentionally small, but uses
//! the same bounded runtime, command IR, delivery planner, and receipts as a
//! network adapter.

use async_trait::async_trait;
use oxidebot_core::{
    event::MessageEvent,
    source::{
        message::{DeliveryPlan, DeliveryReport, Message},
        user::User,
    },
    BotCapabilities, BotId, CallApiTrait, CallResult, ConversationRef, DeliveryReportBuilder,
    Event, EventId, MessageRef, MessageTarget, PlatformId, SupportLevel,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, AdapterMode, BotDescriptor, BotServices,
    IdempotencyGuarantee,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Clone, Debug)]
/// Configuration for the finite stdin/stdout development adapter.
pub struct ConsoleConfig {
    /// Platform-owned identity used for the console bot.
    pub bot_id: String,
    /// Identity assigned to each line read from standard input.
    pub user_id: String,
    /// Direct-conversation identity assigned to console input.
    pub conversation_id: String,
    /// Whether startup guidance is printed before the adapter reads input.
    pub prompt: bool,
}

impl Default for ConsoleConfig {
    fn default() -> Self {
        Self {
            bot_id: "console-bot".to_owned(),
            user_id: "console-user".to_owned(),
            conversation_id: "console".to_owned(),
            prompt: true,
        }
    }
}

impl ConsoleConfig {
    /// Creates the default local development configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    /// Replaces the console bot identity.
    pub fn bot_id(mut self, value: impl Into<String>) -> Self {
        self.bot_id = value.into();
        self
    }

    #[must_use]
    /// Replaces the simulated input-user identity.
    pub fn user_id(mut self, value: impl Into<String>) -> Self {
        self.user_id = value.into();
        self
    }

    #[must_use]
    /// Replaces the simulated direct-conversation identity.
    pub fn conversation_id(mut self, value: impl Into<String>) -> Self {
        self.conversation_id = value.into();
        self
    }

    #[must_use]
    /// Controls whether the adapter prints its interactive startup prompt.
    pub const fn prompt(mut self, enabled: bool) -> Self {
        self.prompt = enabled;
        self
    }
}

#[derive(Clone, Default)]
/// Outbound console API that renders planned messages to standard output.
pub struct ConsoleApi {
    next_message: Arc<AtomicU64>,
}

#[async_trait]
impl CallApiTrait for ConsoleApi {
    fn bot_capabilities(&self) -> BotCapabilities {
        let mut capabilities = BotCapabilities::default();
        capabilities.content.plain_text = SupportLevel::Native;
        capabilities.delivery.replies = SupportLevel::Native;
        capabilities.conversations.direct = SupportLevel::Native;
        capabilities
    }

    async fn send_delivery_plan(
        &self,
        target: MessageTarget,
        plan: DeliveryPlan,
    ) -> CallResult<DeliveryReport> {
        let mut report = DeliveryReportBuilder::new(&plan);
        for message in plan.messages {
            let rendered = message.get_raw_text();
            if rendered.is_empty() {
                println!("[bot -> {target:?}] {message:?}");
            } else {
                println!("[bot -> {target:?}] {rendered}");
            }
            let id = self.next_message.fetch_add(1, Ordering::Relaxed) + 1;
            let sent = MessageRef::new(format!("console-{id}"))
                .in_conversation(target.conversation.clone());
            report.delivered([sent]);
        }
        report.finish()
    }
}

/// Finite stdin/stdout adapter intended for local development and examples.
pub struct ConsoleAdapter {
    config: ConsoleConfig,
    platform: PlatformId,
    bot: BotId,
    api: ConsoleApi,
}

impl ConsoleAdapter {
    /// Creates a console adapter after validating the configured identifiers.
    pub fn new(config: ConsoleConfig) -> std::result::Result<Self, oxidebot_core::InvalidId> {
        Ok(Self {
            platform: PlatformId::new("console")?,
            bot: BotId::new(config.bot_id.clone())?,
            config,
            api: ConsoleApi::default(),
        })
    }

    /// Creates the default interactive development adapter.
    pub fn development() -> Self {
        Self::new(ConsoleConfig::default()).expect("default console identifiers are valid")
    }

    /// Creates a development adapter with one explicit bot identity.
    pub fn named(bot_id: impl Into<String>) -> std::result::Result<Self, oxidebot_core::InvalidId> {
        Self::new(ConsoleConfig::new().bot_id(bot_id))
    }
}

impl Default for ConsoleAdapter {
    fn default() -> Self {
        Self::development()
    }
}

#[async_trait]
impl Adapter for ConsoleAdapter {
    fn descriptor(&self) -> BotDescriptor {
        BotDescriptor::new(self.platform.clone(), self.bot.clone()).display_name("OxideBot Console")
    }

    fn services(&self) -> BotServices {
        BotServices::new(Arc::new(self.api.clone()))
            .send_idempotency(IdempotencyGuarantee::AdapterEmulated)
            .idempotent_delete(false)
    }

    fn mode(&self) -> AdapterMode {
        // Stdin is a finite stream: EOF is a clean application completion,
        // while an interactive terminal naturally remains open until Ctrl-D
        // or process shutdown.
        AdapterMode::Finite
    }

    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        if self.config.prompt {
            println!("OxideBot console is ready. Type a message, or press Ctrl-C to stop.");
        }
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        let mut sequence = 0_u64;
        loop {
            tokio::select! {
                () = context.shutdown().cancelled() => break,
                line = lines.next_line() => {
                    let line = line.map_err(|error| {
                        AdapterError::new(format!("console input failed: {error}"))
                    })?;
                    let Some(line) = line else { break };
                    sequence = sequence.saturating_add(1);
                    let event_id = EventId::new(format!("console-event-{sequence}"))
                        .map_err(|error| AdapterError::new(error.to_string()))?;
                    let mut message = Message::text(line);
                    message.id = format!("console-message-{sequence}");
                    context.submit_event(
                        event_id,
                        Event::Message(MessageEvent {
                            id: message.id.clone(),
                            time: None,
                            sender: User {
                                id: self.config.user_id.clone(),
                                ..User::default()
                            },
                            conversation: ConversationRef::direct(
                                self.config.conversation_id.clone(),
                            ),
                            message,
                        }),
                    ).await?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delivery_reports_every_console_message() {
        let api = ConsoleApi::default();
        let target = MessageTarget::new(ConversationRef::direct("console-user"));
        let plan = DeliveryPlan {
            messages: vec![Message::text("one"), Message::text("two")],
            degradations: Vec::new(),
        };

        let report = api
            .send_delivery_plan(target, plan.clone())
            .await
            .expect("console delivery succeeds");

        oxidebot_testkit::adapter_contract::assert_complete(&plan, &report)
            .expect("console report satisfies the adapter delivery contract");
        assert_eq!(report.messages.len(), 2);
        assert_eq!(report.messages[0].id, "console-1");
        assert_eq!(report.messages[1].id, "console-2");
    }
}
