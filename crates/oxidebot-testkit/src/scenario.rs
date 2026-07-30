//! High-level deterministic end-to-end bot scenarios.
//!
//! This layer composes the lower-level scripted transport with fluent incoming
//! events and human-readable output assertions.

use super::{
    InteractionCall, InteractionFrame, ScriptStep, ScriptedAdapter, SentMessage, TestFrame,
};
use oxidebot_core::{source::message::Message, BotId, EventId, PlatformId};
use std::{sync::Arc, time::Duration};

/// One high-level expectation in a [`BotTest`] script.
#[derive(Clone, Debug)]
pub enum ReplyExpectation {
    /// Requires exact raw-text equality.
    Exact(String),
    /// Requires raw text to contain the supplied substring.
    Contains(String),
    /// Requires complete message-model equality.
    Message(Box<Message>),
}

/// One expected interaction-side effect in call order.
#[derive(Clone, Debug)]
pub enum InteractionExpectation {
    /// Requires an initial acknowledgement, message, or defer response.
    Acknowledged,
    /// Requires an original-response edit containing the supplied text.
    EditContains(String),
    /// Requires a follow-up response containing the supplied text.
    FollowupContains(String),
}

/// Result of a completed high-level Bot test.
#[derive(Clone, Debug)]
pub struct BotTestReport {
    /// Successful messages captured during the scenario.
    pub sent: Vec<SentMessage>,
    /// Interaction side effects captured during the scenario.
    pub interactions: Vec<InteractionCall>,
    /// Total attempted deliveries, including injected temporary failures.
    pub attempts: usize,
}

/// Concise deterministic test harness layered over [`ScriptedAdapter`].
///
/// It is intentionally a convenience API: the lower-level frame and adapter
/// fixtures remain available for admission, retry, and scheduling tests.
pub struct BotTest<S = ()>
where
    S: Send + Sync + 'static,
{
    state: S,
    module: oxidebot_runtime::Module<S>,
    steps: Vec<ScriptStep>,
    expectations: Vec<ReplyExpectation>,
    interaction_expectations: Vec<InteractionExpectation>,
    next_event: u64,
    pause: Duration,
    expectation_timeout: Duration,
    conversation: Arc<str>,
    actor: Arc<str>,
}

impl BotTest<()> {
    /// Creates an empty test application. Add generated commands or configured
    /// features with [`BotTest::add`].
    #[must_use]
    pub fn empty() -> Self {
        Self::with_state((), oxidebot_runtime::Module::new())
    }

    /// Creates a test harness around one complete stateless module.
    #[must_use]
    pub fn new(module: oxidebot_runtime::Module<()>) -> Self {
        Self::with_state((), module)
    }

    /// Creates a stateless test harness from one feature.
    #[must_use]
    pub fn feature<F>(feature: F) -> Self
    where
        F: oxidebot_runtime::IntoFeature<()>,
    {
        Self::empty().add(feature)
    }
}

impl<S> BotTest<S>
where
    S: Send + Sync + 'static,
{
    /// Creates a scenario with explicit root state and a complete module.
    #[must_use]
    pub fn with_state(state: S, module: oxidebot_runtime::Module<S>) -> Self {
        Self {
            state,
            module,
            steps: Vec::new(),
            expectations: Vec::new(),
            interaction_expectations: Vec::new(),
            next_event: 1,
            pause: Duration::ZERO,
            expectation_timeout: Duration::from_secs(2),
            conversation: Arc::from("test-room"),
            actor: Arc::from("test-user"),
        }
    }

    /// Adds one generated command, locally configured feature, or module.
    #[must_use]
    #[allow(
        clippy::should_implement_trait,
        reason = "`add` installs a feature into this fluent test builder; it is not arithmetic"
    )]
    pub fn add<F>(mut self, feature: F) -> Self
    where
        F: oxidebot_runtime::IntoFeature<S>,
    {
        self.module = self.module.add(feature);
        self
    }

    /// Includes another flat module in the test application.
    #[must_use]
    pub fn include(mut self, module: oxidebot_runtime::Module<S>) -> Self {
        self.module = self.module.include(module);
        self
    }

    /// Adds a delay after each generated message or interaction step.
    #[must_use]
    pub fn settle_for(mut self, duration: Duration) -> Self {
        self.pause = duration;
        self
    }

    /// Sets the timeout used by reply expectations as deterministic transport
    /// barriers. The default is two seconds.
    #[must_use]
    pub fn expect_within(mut self, duration: Duration) -> Self {
        self.expectation_timeout = duration;
        self
    }

    /// Replaces the default conversation identity used by generated events.
    #[must_use]
    pub fn in_conversation(mut self, conversation: impl Into<Arc<str>>) -> Self {
        self.conversation = conversation.into();
        self
    }

    /// Replaces the default sender identity used by generated events.
    #[must_use]
    #[allow(
        clippy::wrong_self_convention,
        reason = "`as_user` is an established fluent scenario-builder phrase"
    )]
    pub fn as_user(mut self, actor: impl Into<Arc<str>>) -> Self {
        self.actor = actor.into();
        self
    }

    /// Adds one incoming text message to the scenario.
    #[must_use]
    pub fn message(mut self, text: impl Into<Arc<str>>) -> Self {
        let sequence = self.next_event;
        self.next_event = self.next_event.saturating_add(1);
        let id = EventId::new(format!("test-event-{sequence}"))
            .expect("generated test event id is valid");
        self.steps.push(ScriptStep::Frame(TestFrame::message(
            id,
            self.conversation.as_ref(),
            self.actor.as_ref(),
            sequence,
            text,
        )));
        if !self.pause.is_zero() {
            self.steps.push(ScriptStep::Pause(self.pause));
        }
        self
    }

    /// Adds an answerable button-click interaction to the scenario.
    #[must_use]
    pub fn click(mut self, action_id: impl Into<Arc<str>>) -> Self {
        self.push_click(action_id.into(), Duration::from_secs(5));
        self
    }

    /// Adds a click with an explicit platform acknowledgement deadline.
    #[must_use]
    pub fn click_with_deadline(
        mut self,
        action_id: impl Into<Arc<str>>,
        deadline_after: Duration,
    ) -> Self {
        self.push_click(action_id.into(), deadline_after);
        self
    }

    fn push_click(&mut self, action_id: Arc<str>, deadline_after: Duration) {
        let sequence = self.next_event;
        self.next_event = self.next_event.saturating_add(1);
        let id = EventId::new(format!("test-interaction-{sequence}"))
            .expect("generated test interaction id is valid");
        self.steps.push(ScriptStep::Interaction(
            InteractionFrame::click(
                id,
                self.conversation.as_ref(),
                self.actor.as_ref(),
                action_id,
            )
            .deadline_after(deadline_after),
        ));
        if !self.pause.is_zero() {
            self.steps.push(ScriptStep::Pause(self.pause));
        }
    }

    /// Adds an explicit pause to a scenario without changing the default
    /// settling delay used after each message.
    #[must_use]
    pub fn pause(mut self, duration: Duration) -> Self {
        self.steps.push(ScriptStep::Pause(duration));
        self
    }

    fn push_expectation(mut self, expectation: ReplyExpectation) -> Self {
        self.expectations.push(expectation);
        self.steps.push(ScriptStep::WaitForSent {
            count: self.expectations.len(),
            timeout: self.expectation_timeout,
        });
        self
    }

    /// Expects one exact-text reply after the preceding scenario steps.
    #[must_use]
    pub fn expect_reply(self, text: impl Into<String>) -> Self {
        self.push_expectation(ReplyExpectation::Exact(text.into()))
    }

    /// Expects one reply containing the supplied text.
    #[must_use]
    pub fn expect_reply_contains(self, text: impl Into<String>) -> Self {
        self.push_expectation(ReplyExpectation::Contains(text.into()))
    }

    /// Expects one reply equal to the complete portable message model.
    #[must_use]
    pub fn expect_message(self, message: Message) -> Self {
        self.push_expectation(ReplyExpectation::Message(Box::new(message)))
    }

    fn push_interaction_expectation(mut self, expectation: InteractionExpectation) -> Self {
        self.interaction_expectations.push(expectation);
        self.steps.push(ScriptStep::WaitForInteractions {
            count: self.interaction_expectations.len(),
            timeout: self.expectation_timeout,
        });
        self
    }

    /// Expects any successful initial interaction acknowledgement, including
    /// an immediate message or defer response.
    #[must_use]
    pub fn expect_interaction_ack(self) -> Self {
        self.push_interaction_expectation(InteractionExpectation::Acknowledged)
    }

    /// Expects an original interaction-response edit containing the supplied text.
    #[must_use]
    pub fn expect_edit_contains(self, text: impl Into<String>) -> Self {
        self.push_interaction_expectation(InteractionExpectation::EditContains(text.into()))
    }

    /// Expects an interaction follow-up containing the supplied text.
    #[must_use]
    pub fn expect_followup_contains(self, text: impl Into<String>) -> Self {
        self.push_interaction_expectation(InteractionExpectation::FollowupContains(text.into()))
    }

    /// Explicitly documents that a scenario must not send any messages.
    #[must_use]
    pub fn expect_no_reply(self) -> Self {
        assert!(
            self.expectations.is_empty(),
            "expect_no_reply cannot follow reply expectations",
        );
        self
    }

    /// Runs the complete finite scenario and verifies all declared expectations.
    pub async fn run(self) -> Result<BotTestReport, String> {
        let platform = PlatformId::new("test").expect("static test platform id is valid");
        let bot = BotId::new("bot").expect("static test bot id is valid");
        let (adapter, api) = ScriptedAdapter::new(platform, bot, self.steps);
        oxidebot_runtime::OxideBot::with_state(self.state)
            .adapter(adapter)
            .include(self.module)
            .run_to_completion()
            .await
            .map_err(|error| error.to_string())?;
        let sent = api.sent();
        let interactions = api.interactions();
        if sent.len() != self.expectations.len() {
            return Err(format!(
                "expected {} replies, observed {}: {:?}",
                self.expectations.len(),
                sent.len(),
                sent,
            ));
        }
        for (index, (actual, expected)) in sent.iter().zip(&self.expectations).enumerate() {
            let actual_message = actual.message.clone();
            let actual_text = actual_message.get_raw_text();
            let matches = match expected {
                ReplyExpectation::Exact(expected) => actual_text == *expected,
                ReplyExpectation::Contains(expected) => actual_text.contains(expected),
                ReplyExpectation::Message(expected) => &actual_message == expected.as_ref(),
            };
            if !matches {
                return Err(format!(
                    "reply {} did not match {:?}; actual message was {:?}",
                    index + 1,
                    expected,
                    actual_message,
                ));
            }
        }
        if interactions.len() != self.interaction_expectations.len() {
            return Err(format!(
                "expected {} interaction calls, observed {}: {:?}",
                self.interaction_expectations.len(),
                interactions.len(),
                interactions,
            ));
        }
        for (index, (actual, expected)) in interactions
            .iter()
            .zip(&self.interaction_expectations)
            .enumerate()
        {
            let matches = match (actual, expected) {
                (InteractionCall::Answer { .. }, InteractionExpectation::Acknowledged) => true,
                (
                    InteractionCall::Edit { message, .. },
                    InteractionExpectation::EditContains(expected),
                ) => message.get_raw_text().contains(expected),
                (
                    InteractionCall::Followup { message, .. },
                    InteractionExpectation::FollowupContains(expected),
                ) => message.get_raw_text().contains(expected),
                _ => false,
            };
            if !matches {
                return Err(format!(
                    "interaction call {} did not match {:?}; actual call was {:?}",
                    index + 1,
                    expected,
                    actual,
                ));
            }
        }
        Ok(BotTestReport {
            sent,
            interactions,
            attempts: api.attempts(),
        })
    }
}
