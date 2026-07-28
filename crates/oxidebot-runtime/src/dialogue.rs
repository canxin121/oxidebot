use crate::{
    authoring::ErasedDeliveryPipeline, AskOptions, HandlerError, SessionKey, SessionPolicy,
    SessionRegistry,
};
use oxidebot_core::{
    conversation::MessageTarget, event::Event, source::message::Message, BotObject,
    ConversationKey, FallbackPolicy, SessionNamespace, UserKey,
};
use std::{str::FromStr, sync::Arc, time::Duration};

/// A bounded one-user dialogue scoped to the current conversation and actor.
#[derive(Clone)]
pub struct Dialogue {
    api: BotObject,
    sessions: SessionRegistry,
    conversation: ConversationKey,
    actor: UserKey,
    target: MessageTarget,
    options: AskOptions,
    pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
}

impl Dialogue {
    pub(crate) fn new(
        api: BotObject,
        sessions: SessionRegistry,
        conversation: ConversationKey,
        actor: UserKey,
        target: MessageTarget,
        pipeline: Option<Arc<dyn ErasedDeliveryPipeline>>,
    ) -> Self {
        Self {
            api,
            sessions,
            conversation,
            actor,
            target,
            pipeline,
            options: AskOptions::new(Duration::from_secs(60)).namespace(
                SessionNamespace::new("dialogue").expect("static dialogue namespace is valid"),
            ),
        }
    }

    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = timeout;
        self
    }

    pub fn namespace(mut self, namespace: impl Into<Arc<str>>) -> Result<Self, HandlerError> {
        self.options.namespace = SessionNamespace::new(namespace.into())
            .map_err(|error| HandlerError::Parse(error.to_string()))?;
        Ok(self)
    }

    #[must_use]
    pub const fn policy(mut self, policy: SessionPolicy) -> Self {
        self.options.policy = policy;
        self
    }

    pub async fn wait(&self) -> Result<crate::SessionEvent, HandlerError> {
        let key = SessionKey::new(
            self.conversation.clone(),
            self.actor.clone(),
            self.options.namespace.clone(),
        );
        Ok(self
            .sessions
            .register(key, self.options.timeout, self.options.policy)
            .await?
            .wait()
            .await?)
    }

    pub async fn ask_message(
        &self,
        prompt: impl Into<Message>,
    ) -> Result<crate::SessionEvent, HandlerError> {
        let key = SessionKey::new(
            self.conversation.clone(),
            self.actor.clone(),
            self.options.namespace.clone(),
        );
        let waiter = self
            .sessions
            .register(key, self.options.timeout, self.options.policy)
            .await?;
        let prompt = prompt.into();
        if let Some(pipeline) = &self.pipeline {
            pipeline
                .deliver(&self.api, self.target.clone(), prompt, FallbackPolicy::Auto)
                .await?;
        } else {
            self.api
                .send_outgoing_message_with(self.target.clone(), prompt, FallbackPolicy::Auto)
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
        }
        Ok(waiter.wait().await?)
    }

    pub async fn ask_text(&self, prompt: impl Into<Message>) -> Result<String, HandlerError> {
        let event = self.ask_message(prompt).await?;
        match event.event() {
            Event::MessageEvent(event) => Ok(event.message.get_raw_text()),
            _ => Err(HandlerError::Parse(
                "dialogue response is not a message event".into(),
            )),
        }
    }

    pub async fn ask<T>(&self, prompt: impl Into<Message>) -> Result<T, HandlerError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        self.ask_text(prompt)
            .await?
            .trim()
            .parse()
            .map_err(|error: T::Err| HandlerError::Parse(error.to_string()))
    }

    pub async fn confirm(&self, prompt: impl Into<Message>) -> Result<bool, HandlerError> {
        let value = self.ask_text(prompt).await?;
        match value.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" | "true" | "1" | "ok" | "确认" | "是" | "好" => Ok(true),
            "n" | "no" | "false" | "0" | "cancel" | "取消" | "否" | "不" => Ok(false),
            _ => Err(HandlerError::Parse("expected a yes/no confirmation".into())),
        }
    }

    pub async fn choose<T, I, L>(
        &self,
        prompt: impl Into<Message>,
        choices: I,
    ) -> Result<T, HandlerError>
    where
        T: Clone,
        I: IntoIterator<Item = (L, T)>,
        L: Into<String>,
    {
        let choices = choices
            .into_iter()
            .map(|(label, value)| (label.into(), value))
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return Err(HandlerError::Parse("choice list cannot be empty".into()));
        }
        let mut rendered = prompt.into();
        for (index, (label, _)) in choices.iter().enumerate() {
            rendered = rendered.then(format!("\n{}. {label}", index + 1));
        }
        let answer = self.ask_text(rendered).await?;
        if let Ok(index) = answer.trim().parse::<usize>() {
            if let Some((_, value)) = choices.get(index.saturating_sub(1)) {
                return Ok(value.clone());
            }
        }
        choices
            .iter()
            .find(|(label, _)| label.eq_ignore_ascii_case(answer.trim()))
            .map(|(_, value)| value.clone())
            .ok_or_else(|| HandlerError::Parse("unknown choice".into()))
    }
}

impl std::fmt::Debug for Dialogue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Dialogue")
            .field("conversation", &self.conversation)
            .field("actor", &self.actor)
            .field("target", &self.target)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}
