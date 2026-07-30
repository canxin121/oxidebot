//! Handler-local localization facade.
//!
//! The facade binds the application's translation catalog and locale resolver
//! to one handler context, while the catalog itself stays in core.

use crate::{Context, Extract, ExtractError, HandlerError, HandlerResult};
use async_trait::async_trait;
use futures_util::future::BoxFuture;
use oxidebot_core::{source::message::Message, LocalizedMessage, TemplateValue};
use std::sync::Arc;

/// Handler-local localization facade backed by the application's bounded
/// [`TranslationCatalog`] and locale resolver.
#[derive(Clone)]
pub struct I18n {
    inner: Arc<dyn ErasedI18n>,
}

#[async_trait]
trait ErasedI18n: Send + Sync + 'static {
    async fn render(&self, message: LocalizedMessage) -> HandlerResult<Message>;
}

struct BoundI18n<S>
where
    S: Send + Sync + 'static,
{
    context: Context<S>,
}

#[async_trait]
impl<S> ErasedI18n for BoundI18n<S>
where
    S: Send + Sync + 'static,
{
    async fn render(&self, message: LocalizedMessage) -> HandlerResult<Message> {
        let catalog = self
            .context
            .authoring()
            .translations
            .as_ref()
            .ok_or_else(|| HandlerError::internal("no TranslationCatalog is configured"))?;
        let locale = self.context.authoring().locale(&self.context).await;
        message
            .render(catalog, locale.as_deref())
            .map_err(|error| HandlerError::internal(error.to_string()))
    }
}

impl I18n {
    /// Starts an awaitable localized message identified by `key`.
    #[must_use]
    pub fn message(&self, key: impl Into<Arc<str>>) -> I18nMessage {
        I18nMessage {
            i18n: self.clone(),
            message: LocalizedMessage::new(key),
        }
    }

    /// Renders a localized message using the current handler locale.
    pub async fn render(&self, message: LocalizedMessage) -> HandlerResult<Message> {
        self.inner.render(message).await
    }
}

impl std::fmt::Debug for I18n {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("I18n").finish_non_exhaustive()
    }
}

/// Awaitable localized message builder.
#[derive(Clone)]
pub struct I18nMessage {
    i18n: I18n,
    message: LocalizedMessage,
}

impl I18nMessage {
    /// Supplies one template argument.
    #[must_use]
    pub fn arg(mut self, name: impl Into<Arc<str>>, value: impl Into<TemplateValue>) -> Self {
        self.message = self.message.arg(name, value);
        self
    }

    /// Renders this localized message with its accumulated arguments.
    pub async fn render(self) -> HandlerResult<Message> {
        self.i18n.render(self.message).await
    }
}

impl std::future::IntoFuture for I18nMessage {
    type Output = HandlerResult<Message>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.render())
    }
}

impl<S> Extract<S> for I18n
where
    S: Send + Sync + 'static,
{
    fn extract(context: &Context<S>) -> Result<Self, ExtractError> {
        Ok(Self {
            inner: Arc::new(BoundI18n {
                context: context.clone(),
            }),
        })
    }
}
