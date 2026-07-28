use oxidebot_core::source::message::{Message, MessageSegment};
use std::{borrow::Cow, convert::Infallible, fmt};

/// The result of one handler or middleware call.
///
/// A response can enqueue zero or more canonical 0.1.8 messages and decide
/// whether routing should stop. Message-like return values stop routing by
/// default, while `()` and `None` continue it.
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub(crate) stop: bool,
    pub(crate) replies: Vec<Vec<MessageSegment>>,
}

impl Response {
    #[must_use]
    pub const fn continue_() -> Self {
        Self {
            stop: false,
            replies: Vec::new(),
        }
    }

    #[must_use]
    pub const fn stop() -> Self {
        Self {
            stop: true,
            replies: Vec::new(),
        }
    }

    #[must_use]
    pub fn reply(mut self, message: impl Into<Message>) -> Self {
        self.replies.push(message.into().into_segments());
        self
    }

    #[must_use]
    pub fn text(self, text: impl Into<String>) -> Self {
        self.reply(Message::text(text))
    }

    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self::stop().text(text)
    }

    #[must_use]
    pub const fn is_stopped(&self) -> bool {
        self.stop
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.replies.is_empty()
    }

    #[must_use]
    pub fn replies(&self) -> impl ExactSizeIterator<Item = &[MessageSegment]> {
        self.replies.iter().map(Vec::as_slice)
    }

    #[must_use]
    pub fn and(mut self, mut next: Self) -> Self {
        self.stop |= next.stop;
        self.replies.append(&mut next.replies);
        self
    }

    #[must_use]
    pub const fn with_stop(mut self, stop: bool) -> Self {
        self.stop = stop;
        self
    }
}

/// Converts ordinary handler return values into a routing response.
pub trait IntoResponse {
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::continue_()
    }
}

impl IntoResponse for Infallible {
    fn into_response(self) -> Response {
        match self {}
    }
}

impl IntoResponse for Message {
    fn into_response(self) -> Response {
        Response::stop().reply(self)
    }
}

impl IntoResponse for MessageSegment {
    fn into_response(self) -> Response {
        Message::from(self).into_response()
    }
}

impl IntoResponse for Vec<MessageSegment> {
    fn into_response(self) -> Response {
        Message::from(self).into_response()
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        Message::text(self).into_response()
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        Message::text(self).into_response()
    }
}

impl IntoResponse for Cow<'static, str> {
    fn into_response(self) -> Response {
        Message::text(self.into_owned()).into_response()
    }
}

impl<T> IntoResponse for Option<T>
where
    T: IntoResponse,
{
    fn into_response(self) -> Response {
        self.map_or_else(Response::continue_, IntoResponse::into_response)
    }
}

impl<T, E> IntoResponse for Result<T, E>
where
    T: IntoResponse,
    E: fmt::Display,
{
    fn into_response(self) -> Response {
        match self {
            Ok(value) => value.into_response(),
            Err(error) => Response::error(error.to_string()),
        }
    }
}

/// Backwards-compatible name for the pre-router result type.
pub type Outcome = Response;
