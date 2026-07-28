use crate::{
    authoring::ErasedDeliveryPipeline, AskOptions, HandlerError, SessionKey, SessionPolicy,
    SessionRegistry,
};
use futures_util::future::BoxFuture;
use oxidebot_core::{
    conversation::MessageTarget,
    event::Event,
    source::message::{Message, MessageSegment},
    BotObject, ConversationKey, FallbackPolicy, SessionNamespace, UserKey,
};
use std::{fmt::Display, future::IntoFuture, str::FromStr, sync::Arc, time::Duration};

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

    /// Uses a compile-time/static namespace without interrupting a builder
    /// chain. Invalid static values indicate a programming error.
    #[must_use]
    pub fn named(mut self, namespace: &'static str) -> Self {
        self.options.namespace = SessionNamespace::new(namespace)
            .expect("static dialogue namespace must contain 1..=1024 bytes");
        self
    }

    /// Uses a dynamically supplied namespace and validates the external value.
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

    /// Starts a typed, retryable, validated question.
    #[must_use]
    pub fn question<T>(&self, prompt: impl Into<Message>) -> DialogueQuestion<T>
    where
        T: FromStr + Send + 'static,
        T::Err: Display + Send + 'static,
    {
        DialogueQuestion::new(self.clone(), prompt.into())
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
        T: FromStr + Send + 'static,
        T::Err: Display + Send + 'static,
    {
        self.question(prompt).attempts(1).run().await
    }

    /// Collects a reusable typed form generated with
    /// `#[derive(oxidebot::DialogueForm)]` or implemented manually.
    pub async fn form<T>(&self) -> Result<T, HandlerError>
    where
        T: DialogueForm,
    {
        T::collect(self.clone()).await
    }

    pub async fn confirm(&self, prompt: impl Into<Message>) -> Result<bool, HandlerError> {
        self.confirm_with(prompt, ConfirmationWords::default(), 3)
            .await
    }

    pub async fn confirm_with(
        &self,
        prompt: impl Into<Message>,
        words: ConfirmationWords,
        attempts: usize,
    ) -> Result<bool, HandlerError> {
        let mut prompt = prompt.into();
        let attempts = attempts.max(1);
        for attempt in 0..attempts {
            let answer = self.ask_text(words.decorate(prompt)).await?;
            if let Some(value) = words.parse(&answer) {
                return Ok(value);
            }
            if attempt + 1 == attempts {
                break;
            }
            prompt = Message::text(words.retry_message.as_ref());
        }
        Err(HandlerError::user(words.retry_message.as_ref()))
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
        self.choose_with(prompt, choices, 3).await
    }

    pub async fn choose_with<T, I, L>(
        &self,
        prompt: impl Into<Message>,
        choices: I,
        attempts: usize,
    ) -> Result<T, HandlerError>
    where
        T: Clone,
        I: IntoIterator<Item = (L, T)>,
        L: Into<String>,
    {
        self.choose_with_error(
            prompt,
            choices,
            attempts,
            Message::text("未知选项，请输入编号、选项名称或使用按钮。"),
        )
        .await
    }

    pub async fn choose_with_error<T, I, L>(
        &self,
        prompt: impl Into<Message>,
        choices: I,
        attempts: usize,
        error_prompt: impl Into<Message>,
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
        // Buttons are carried by the unified message IR. Platforms without
        // components use the normal delivery fallback and retain the numbered
        // text choices above.
        rendered = rendered.buttons(choices.iter().enumerate().map(|(index, (label, _))| {
            oxidebot_core::Button::send_text(label.clone(), (index + 1).to_string())
        }));

        let attempts = attempts.max(1);
        let error_prompt = error_prompt.into();
        let mut current_prompt = rendered;
        for attempt in 0..attempts {
            let answer = self.ask_text(current_prompt).await?;
            if let Ok(index) = answer.trim().parse::<usize>() {
                if let Some((_, value)) = choices.get(index.saturating_sub(1)) {
                    return Ok(value.clone());
                }
            }
            if let Some((_, value)) = choices
                .iter()
                .find(|(label, _)| label.eq_ignore_ascii_case(answer.trim()))
            {
                return Ok(value.clone());
            }
            if attempt + 1 == attempts {
                break;
            }
            current_prompt = rendered_with_prefix(&error_prompt, &choices);
        }
        let message = error_prompt.get_raw_text();
        Err(HandlerError::user(if message.trim().is_empty() {
            "No valid choice was selected."
        } else {
            message.as_str()
        }))
    }
}

/// Boxed future used by [`DialogueForm`] implementations. Form collection is
/// a cold, explicitly interactive path, so boxing here does not affect normal
/// message or command dispatch.
pub type DialogueFormFuture<T> = BoxFuture<'static, Result<T, HandlerError>>;

/// A reusable, strongly typed multi-question dialogue form.
pub trait DialogueForm: Sized + Send + 'static {
    fn collect(dialogue: Dialogue) -> DialogueFormFuture<Self>;
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

/// Locale-customizable confirmation vocabulary.
#[derive(Clone, Debug)]
pub struct ConfirmationWords {
    yes: Arc<[Arc<str>]>,
    no: Arc<[Arc<str>]>,
    retry_message: Arc<str>,
}

impl ConfirmationWords {
    #[must_use]
    pub fn new<Y, N, YV, NV>(yes: Y, no: N) -> Self
    where
        Y: IntoIterator<Item = YV>,
        N: IntoIterator<Item = NV>,
        YV: Into<Arc<str>>,
        NV: Into<Arc<str>>,
    {
        Self {
            yes: yes.into_iter().map(Into::into).collect::<Vec<_>>().into(),
            no: no.into_iter().map(Into::into).collect::<Vec<_>>().into(),
            retry_message: Arc::from("请输入 yes/no、是/否或使用按钮。"),
        }
    }

    #[must_use]
    pub fn retry_message(mut self, value: impl Into<Arc<str>>) -> Self {
        self.retry_message = value.into();
        self
    }

    fn parse(&self, value: &str) -> Option<bool> {
        let value = value.trim();
        self.yes
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(value))
            .then_some(true)
            .or_else(|| {
                self.no
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(value))
                    .then_some(false)
            })
    }

    fn decorate(&self, prompt: Message) -> Message {
        let yes = self
            .yes
            .first()
            .map_or_else(|| "yes".to_owned(), ToString::to_string);
        let no = self
            .no
            .first()
            .map_or_else(|| "no".to_owned(), ToString::to_string);
        prompt.buttons([
            oxidebot_core::Button::send_text(yes.clone(), yes),
            oxidebot_core::Button::send_text(no.clone(), no),
        ])
    }
}

fn rendered_with_prefix<T>(error: &Message, choices: &[(String, T)]) -> Message {
    let mut message = error.clone();
    for (index, (label, _)) in choices.iter().enumerate() {
        message.push(MessageSegment::text(format!("\n{}. {label}", index + 1)));
    }
    message.buttons(choices.iter().enumerate().map(|(index, (label, _))| {
        oxidebot_core::Button::send_text(label.clone(), (index + 1).to_string())
    }))
}

impl Default for ConfirmationWords {
    fn default() -> Self {
        Self::new(
            ["y", "yes", "true", "1", "ok", "确认", "是", "好"],
            ["n", "no", "false", "0", "cancel", "取消", "否", "不"],
        )
    }
}

type QuestionValidator<T> = Arc<dyn Fn(&T) -> Result<(), Arc<str>> + Send + Sync>;

/// A typed dialogue question with bounded retries and domain validation.
pub struct DialogueQuestion<T>
where
    T: FromStr + Send + 'static,
    T::Err: Display + Send + 'static,
{
    dialogue: Dialogue,
    prompt: Message,
    attempts: usize,
    error_prompt: Message,
    trim: bool,
    validator: Option<QuestionValidator<T>>,
}

impl<T> DialogueQuestion<T>
where
    T: FromStr + Send + 'static,
    T::Err: Display + Send + 'static,
{
    fn new(dialogue: Dialogue, prompt: Message) -> Self {
        Self {
            dialogue,
            prompt,
            attempts: 3,
            error_prompt: Message::text("输入格式不正确，请重试。"),
            trim: true,
            validator: None,
        }
    }

    #[must_use]
    pub fn attempts(mut self, attempts: usize) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    #[must_use]
    pub fn retry(mut self, retries: usize) -> Self {
        self.attempts = retries.saturating_add(1).max(1);
        self
    }

    #[must_use]
    pub fn error(mut self, prompt: impl Into<Message>) -> Self {
        self.error_prompt = prompt.into();
        self
    }

    #[must_use]
    pub const fn trim(mut self, enabled: bool) -> Self {
        self.trim = enabled;
        self
    }

    #[must_use]
    pub fn validate<F>(mut self, validator: F) -> Self
    where
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        self.validator = Some(Arc::new(move |value| {
            validator(value)
                .then_some(())
                .ok_or_else(|| Arc::from("value did not satisfy the dialogue constraint"))
        }));
        self
    }

    #[must_use]
    pub fn try_validate<F, E>(mut self, validator: F) -> Self
    where
        F: Fn(&T) -> Result<(), E> + Send + Sync + 'static,
        E: Display,
    {
        self.validator = Some(Arc::new(move |value| {
            validator(value).map_err(|error| Arc::from(error.to_string()))
        }));
        self
    }

    pub async fn run(self) -> Result<T, HandlerError> {
        let mut prompt = self.prompt;
        let final_error = self.error_prompt.get_raw_text();
        for attempt in 0..self.attempts {
            let answer = self.dialogue.ask_text(prompt).await?;
            let input = if self.trim {
                answer.trim()
            } else {
                answer.as_str()
            };
            let valid = match input.parse::<T>() {
                Ok(value) => match &self.validator {
                    Some(validator) if validator(&value).is_err() => None,
                    _ => Some(value),
                },
                Err(_) => None,
            };
            if let Some(value) = valid {
                return Ok(value);
            }
            if attempt + 1 == self.attempts {
                break;
            }
            prompt = self.error_prompt.clone();
        }
        Err(HandlerError::user(if final_error.trim().is_empty() {
            "Input could not be parsed after the configured attempts."
        } else {
            final_error.as_str()
        }))
    }
}

impl<T> IntoFuture for DialogueQuestion<T>
where
    T: FromStr + Send + 'static,
    T::Err: Display + Send + 'static,
{
    type Output = Result<T, HandlerError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}
