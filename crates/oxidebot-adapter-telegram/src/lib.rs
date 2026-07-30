//! Telegram Bot API long-polling support for OxideBot.
//!
//! The adapter deliberately starts with the portable, production-critical
//! baseline: incoming text messages, capability-aware text delivery, typed
//! transport errors, long-poll cancellation, and a configurable Bot API base
//! URL for sandbox or fixture servers. Unsupported Telegram features remain
//! visible through `BotCapabilities` and `CallError::Unsupported` rather than
//! being silently approximated.

mod render;
mod wire;

use async_trait::async_trait;
use oxidebot_core::{
    conversation::{MessageRef, MessageTarget},
    source::message::{DeliveryPlan, DeliveryReport},
    BotCapabilities, BotId, CallApiTrait, CallError, CallResult, DeliveryReportBuilder, PlatformId,
    SupportLevel,
};
use oxidebot_runtime::{
    Adapter, AdapterContext, AdapterError, BotDescriptor, BotServices, IdempotencyGuarantee,
};
use reqwest::{Client, Url};
use serde::{de::DeserializeOwned, Serialize};
use std::{sync::Arc, time::Duration};
use thiserror::Error;
#[cfg(test)]
use wire::TelegramErrorParameters;
use wire::{
    map_telegram_error, GetUpdatesRequest, SendMessageRequest, TelegramEnvelope,
    TelegramSentMessage, TelegramUpdate,
};

const TELEGRAM_PLATFORM: &str = "telegram";
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const MAX_LONG_POLL_SECONDS: u16 = 50;

/// Configuration validation failure for a [`TelegramAdapter`].
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TelegramConfigError {
    /// The bot token is empty or only whitespace.
    #[error("Telegram bot token must not be empty")]
    EmptyToken,
    /// The Bot API base URL is empty or cannot be parsed by the HTTP client.
    #[error("Telegram Bot API base URL must not be empty")]
    EmptyApiBase,
    /// The Bot API base URL is not an absolute HTTP(S) URL.
    #[error("Telegram Bot API base URL must be an absolute HTTP(S) URL")]
    InvalidApiBase,
    /// The polling timeout exceeds Telegram's Bot API limit.
    #[error("Telegram long-poll timeout must be between 1 and {MAX_LONG_POLL_SECONDS} seconds")]
    InvalidPollTimeout,
    /// The requested update batch size is outside Telegram's supported range.
    #[error("Telegram update limit must be between 1 and 100")]
    InvalidUpdateLimit,
    /// An HTTP client could not be constructed.
    #[error("could not construct Telegram HTTP client: {0}")]
    Client(String),
    /// The configured OxideBot identity is invalid.
    #[error(transparent)]
    InvalidBotId(#[from] oxidebot_core::InvalidId),
}

/// Long-polling configuration for Telegram Bot API.
#[derive(Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    token: Arc<str>,
    bot_id: BotId,
    api_base: Url,
    poll_timeout: u16,
    update_limit: u8,
}

impl std::fmt::Debug for TelegramConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TelegramConfig")
            .field("token", &"<redacted>")
            .field("bot_id", &self.bot_id)
            .field("api_base", &self.api_base)
            .field("poll_timeout", &self.poll_timeout)
            .field("update_limit", &self.update_limit)
            .finish()
    }
}

impl TelegramConfig {
    /// Builds configuration for Telegram's public Bot API endpoint.
    pub fn new(
        token: impl Into<Arc<str>>,
        bot_id: impl Into<Arc<str>>,
    ) -> Result<Self, TelegramConfigError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(TelegramConfigError::EmptyToken);
        }
        Ok(Self {
            token,
            bot_id: BotId::new(bot_id)?,
            api_base: Url::parse(DEFAULT_API_BASE)
                .expect("the static Telegram Bot API base URL is valid"),
            poll_timeout: MAX_LONG_POLL_SECONDS,
            update_limit: 100,
        })
    }

    /// Replaces the Bot API base URL for a compatible server or test fixture.
    pub fn api_base(mut self, value: impl AsRef<str>) -> Result<Self, TelegramConfigError> {
        let value = value.as_ref();
        if value.trim().is_empty() {
            return Err(TelegramConfigError::EmptyApiBase);
        }
        let mut api_base =
            Url::parse(value.trim()).map_err(|_| TelegramConfigError::InvalidApiBase)?;
        if !matches!(api_base.scheme(), "http" | "https") || api_base.host().is_none() {
            return Err(TelegramConfigError::InvalidApiBase);
        }
        if !api_base.path().ends_with('/') {
            let path = format!("{}/", api_base.path());
            api_base.set_path(&path);
        }
        self.api_base = api_base;
        Ok(self)
    }

    /// Sets Telegram's long-poll timeout in whole seconds.
    pub fn poll_timeout(mut self, value: u16) -> Result<Self, TelegramConfigError> {
        if !(1..=MAX_LONG_POLL_SECONDS).contains(&value) {
            return Err(TelegramConfigError::InvalidPollTimeout);
        }
        self.poll_timeout = value;
        Ok(self)
    }

    /// Sets the maximum number of updates requested in one long-poll response.
    pub fn update_limit(mut self, value: u8) -> Result<Self, TelegramConfigError> {
        if !(1..=100).contains(&value) {
            return Err(TelegramConfigError::InvalidUpdateLimit);
        }
        self.update_limit = value;
        Ok(self)
    }
}

/// A real Telegram Bot API adapter using cancellable long polling.
pub struct TelegramAdapter {
    descriptor: BotDescriptor,
    config: TelegramConfig,
    api: TelegramApi,
}

impl TelegramAdapter {
    /// Creates an adapter for Telegram's public Bot API endpoint.
    pub fn new(
        token: impl Into<Arc<str>>,
        bot_id: impl Into<Arc<str>>,
    ) -> Result<Self, TelegramConfigError> {
        Self::with_config(TelegramConfig::new(token, bot_id)?)
    }

    /// Creates an adapter from complete long-polling configuration.
    pub fn with_config(config: TelegramConfig) -> Result<Self, TelegramConfigError> {
        let api = TelegramApi::new(&config)?;
        let platform = PlatformId::new(TELEGRAM_PLATFORM).expect("static Telegram platform id");
        Ok(Self {
            descriptor: BotDescriptor::new(platform, config.bot_id.clone())
                .display_name("Telegram"),
            config,
            api,
        })
    }
}

#[async_trait]
impl Adapter for TelegramAdapter {
    fn descriptor(&self) -> BotDescriptor {
        self.descriptor.clone()
    }

    fn services(&self) -> BotServices {
        BotServices::new(Arc::new(self.api.clone()))
            .send_idempotency(IdempotencyGuarantee::Unsupported)
            .idempotent_delete(false)
    }

    async fn run(self: Box<Self>, context: AdapterContext) -> Result<(), AdapterError> {
        let mut offset = None;
        let mut retry_delay = Duration::from_secs(1);
        loop {
            if context.is_cancelled() {
                return Ok(());
            }
            let updates = tokio::select! {
                () = context.shutdown().cancelled() => return Ok(()),
                result = self.api.get_updates(offset, self.config.poll_timeout, self.config.update_limit) => result,
            };
            match updates {
                Ok(updates) => {
                    retry_delay = Duration::from_secs(1);
                    for update in updates {
                        offset = Some(update.update_id.saturating_add(1));
                        if let Some((id, event)) = update.into_event() {
                            context.submit_event(id, event).await?;
                        }
                    }
                }
                Err(error) => {
                    let delay = error.retry_after().unwrap_or(retry_delay);
                    tracing::warn!(error = %error, retry_after = ?delay, "Telegram getUpdates failed");
                    tokio::select! {
                        () = context.shutdown().cancelled() => return Ok(()),
                        () = tokio::time::sleep(delay) => {}
                    }
                    retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(30));
                }
            }
        }
    }
}

/// Telegram's portable OxideBot API implementation.
#[derive(Clone)]
pub struct TelegramApi {
    client: Client,
    token: Arc<str>,
    api_base: Url,
}

impl TelegramApi {
    fn new(config: &TelegramConfig) -> Result<Self, TelegramConfigError> {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        // Do not consult Rustls's process-global default provider here. A host
        // application can legitimately combine this adapter's `ring` feature
        // with another dependency's `aws-lc-rs` feature, in which case Rustls
        // cannot infer a unique global provider and `ClientConfig::builder()`
        // panics. This transport owns its TLS client, so choose `ring`
        // explicitly for this client only.
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .expect("ring supports Rustls's default TLS protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .use_preconfigured_tls(tls)
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|error| TelegramConfigError::Client(error.to_string()))?;
        Ok(Self {
            client,
            token: Arc::clone(&config.token),
            api_base: config.api_base.clone(),
        })
    }

    fn method_url(&self, method: &str) -> Url {
        self.api_base
            .join(&format!("./bot{}/{method}", self.token))
            .expect("configured Telegram API base URL can resolve a static method path")
    }

    async fn call<T, P>(&self, method: &str, payload: &P) -> CallResult<T>
    where
        T: DeserializeOwned,
        P: Serialize + ?Sized,
    {
        let response = self
            .client
            .post(self.method_url(method))
            .json(payload)
            .send()
            .await
            .map_err(map_transport_error)?;
        let status = response.status();
        let body = response.bytes().await.map_err(map_transport_error)?;
        let envelope: TelegramEnvelope<T> = serde_json::from_slice(&body).map_err(|error| {
            CallError::permanent(format!("Telegram {method} returned invalid JSON: {error}"))
        })?;
        if envelope.ok {
            return envelope.result.ok_or_else(|| {
                CallError::permanent(format!("Telegram {method} succeeded without a result"))
            });
        }
        Err(map_telegram_error(status, envelope))
    }

    async fn get_updates(
        &self,
        offset: Option<i64>,
        timeout: u16,
        limit: u8,
    ) -> CallResult<Vec<TelegramUpdate>> {
        self.call(
            "getUpdates",
            &GetUpdatesRequest {
                offset,
                timeout,
                limit,
            },
        )
        .await
    }
}

#[async_trait]
impl CallApiTrait for TelegramApi {
    fn bot_capabilities(&self) -> BotCapabilities {
        let mut capabilities = BotCapabilities::default();
        capabilities.content.plain_text = SupportLevel::Native;
        capabilities.content.rich_text = SupportLevel::Emulated;
        capabilities.conversations.direct = SupportLevel::Native;
        capabilities.conversations.groups = SupportLevel::Native;
        capabilities
    }

    async fn send_delivery_plan(
        &self,
        target: MessageTarget,
        plan: DeliveryPlan,
    ) -> CallResult<DeliveryReport> {
        let mut report = DeliveryReportBuilder::new(&plan);
        let mut delivered_any = false;
        for message in plan.messages {
            let (text, parse_mode) = render::telegram_text(&message);
            if text.is_empty() {
                let error = CallError::invalid_request(
                    "Telegram cannot send an empty portable text message",
                );
                return if delivered_any {
                    Err(report.failed(error.to_string()))
                } else {
                    Err(error)
                };
            }
            let chat_id = target.conversation.id.to_string();
            let response = self
                .call::<TelegramSentMessage, _>(
                    "sendMessage",
                    &SendMessageRequest {
                        chat_id: &chat_id,
                        text: &text,
                        parse_mode,
                    },
                )
                .await;
            match response {
                Ok(sent) => {
                    report.delivered([MessageRef::new(sent.message_id.to_string())
                        .in_conversation(target.conversation.clone())]);
                    delivered_any = true;
                }
                Err(error) => {
                    return if delivered_any {
                        Err(report.failed(error.to_string()))
                    } else {
                        Err(error)
                    };
                }
            }
        }
        report.finish()
    }
}

fn map_transport_error(error: reqwest::Error) -> CallError {
    if error.is_timeout() {
        CallError::timeout(error.to_string())
    } else if error.is_connect() || error.is_request() {
        CallError::temporary(error.to_string())
    } else {
        CallError::permanent(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidebot_core::{
        content::{RichText, TextStyle},
        conversation::{ConversationKind, ConversationRef},
        source::message::Message,
        Event,
    };
    use reqwest::StatusCode;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn text_update_normalizes_to_a_canonical_message_event() {
        let update: TelegramUpdate = serde_json::from_value(serde_json::json!({
            "update_id": 42,
            "message": {
                "message_id": 7,
                "date": 1_700_000_000,
                "from": { "id": 99, "first_name": "Ada", "last_name": "Lovelace" },
                "chat": { "id": 99, "type": "private" },
                "text": "/ping"
            }
        }))
        .expect("recorded Telegram update is valid JSON");
        let (id, Event::Message(event)) = update.into_event().expect("text update maps") else {
            panic!("expected a canonical message event");
        };
        assert_eq!(id.as_str(), "telegram:update:42");
        assert_eq!(event.sender.id, 99_u64.into());
        assert_eq!(event.conversation.kind, ConversationKind::Direct);
        assert_eq!(event.message.get_raw_text(), "/ping");
    }

    #[test]
    fn telegram_rate_limit_preserves_retry_after() {
        let error = map_telegram_error(
            StatusCode::TOO_MANY_REQUESTS,
            TelegramEnvelope::<serde_json::Value> {
                ok: false,
                result: None,
                error_code: Some(429),
                description: Some("Too Many Requests".into()),
                parameters: Some(TelegramErrorParameters {
                    retry_after: Some(3),
                }),
            },
        );
        assert_eq!(error.retry_after(), Some(Duration::from_secs(3)));
        assert!(error.is_retryable());
    }

    #[test]
    fn request_payload_uses_the_documented_telegram_field_names() {
        let payload = serde_json::to_value(SendMessageRequest {
            chat_id: "-100123",
            text: "hello",
            parse_mode: None,
        })
        .expect("request serializes");
        assert_eq!(payload["chat_id"], "-100123");
        assert_eq!(payload["text"], "hello");
        assert!(payload.get("parse_mode").is_none());
    }

    #[test]
    fn config_debug_redacts_the_bot_token() {
        let config = TelegramConfig::new("123:secret-token", "bot").expect("valid config");
        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-token"));
    }

    #[test]
    fn config_rejects_non_http_api_bases_before_startup() {
        let config = TelegramConfig::new("123:secret-token", "bot")
            .expect("valid config")
            .api_base("not a URL");
        assert_eq!(config, Err(TelegramConfigError::InvalidApiBase));

        let config = TelegramConfig::new("123:secret-token", "bot")
            .expect("valid config")
            .api_base("ftp://fixture.example");
        assert_eq!(config, Err(TelegramConfigError::InvalidApiBase));
    }

    #[tokio::test]
    async fn send_delivery_plan_uses_real_http_transport_and_builds_a_report() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local Telegram fixture listener binds");
        let address = listener
            .local_addr()
            .expect("fixture listener has a local address");
        let fixture = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("fixture accepts request");
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1024];
                let read = stream
                    .read(&mut chunk)
                    .await
                    .expect("fixture reads request");
                assert_ne!(read, 0, "client closed request before headers");
                request.extend_from_slice(&chunk[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..header_end])
                .expect("headers are UTF-8")
                .to_owned();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("request includes content length");
            while request.len().saturating_sub(header_end) < content_length {
                let mut chunk = [0_u8; 1024];
                let read = stream.read(&mut chunk).await.expect("fixture reads body");
                assert_ne!(read, 0, "client closed request before body");
                request.extend_from_slice(&chunk[..read]);
            }
            let body = &request[header_end..header_end + content_length];
            let body: serde_json::Value = serde_json::from_slice(body).expect("body is JSON");
            assert!(headers.starts_with("POST /bot123:test-token/sendMessage HTTP/1.1\r\n"));
            assert_eq!(body["chat_id"], "-100123");
            assert_eq!(body["text"], "*套餐*：主卡\n剩余 `1.20 GB`");
            assert_eq!(body["parse_mode"], "MarkdownV2");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 38\r\nconnection: close\r\n\r\n{\"ok\":true,\"result\":{\"message_id\":77}}",
                )
                .await
                .expect("fixture writes response");
        });

        let config = TelegramConfig::new("123:test-token", "bot")
            .expect("valid configuration")
            .api_base(format!("http://{address}"))
            .expect("fixture URL is valid");
        let api = TelegramApi::new(&config).expect("HTTP client builds");
        let target = MessageTarget::new(ConversationRef::group("-100123"));
        let rich = RichText::plain("套餐：主卡\n剩余 1.20 GB")
            .span(0.."套餐".len(), TextStyle::Bold)
            .span(
                "套餐：主卡\n剩余 ".len().."套餐：主卡\n剩余 1.20 GB".len(),
                TextStyle::Code,
            );
        let plan = DeliveryPlan {
            messages: vec![Message::rich_text(rich)],
            degradations: Vec::new(),
        };
        let report = api
            .send_delivery_plan(target, plan)
            .await
            .expect("fixture accepts Telegram delivery");
        fixture.await.expect("fixture task completes");
        assert_eq!(report.messages.len(), 1);
        assert_eq!(report.messages[0].id, "77".into());
        oxidebot_testkit::adapter_contract::assert_complete(
            &DeliveryPlan {
                messages: vec![Message::rich_text(
                    RichText::plain("套餐：主卡\n剩余 1.20 GB")
                        .span(0.."套餐".len(), TextStyle::Bold)
                        .span(
                            "套餐：主卡\n剩余 ".len().."套餐：主卡\n剩余 1.20 GB".len(),
                            TextStyle::Code,
                        ),
                )],
                degradations: Vec::new(),
            },
            &report,
        )
        .expect("Telegram report satisfies the adapter delivery contract");
    }
}
