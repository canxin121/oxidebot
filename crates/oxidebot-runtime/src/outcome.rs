use oxidebot_core::source::message::{Message, MessageSegment};
use std::{borrow::Cow, convert::Infallible};

/// Whether one handler inherits or overrides its kind's normal event flow.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Propagation {
    /// Use the natural default for the matched handler kind.
    #[default]
    Inherit,
    /// Allow later matching handlers to run.
    Continue,
    /// Prevent later matching handlers from running.
    Stop,
}

/// Effects produced by one Bot handler.
///
/// An outcome can enqueue canonical 0.1.8 messages and optionally override the
/// handler's normal propagation policy. Commands and interactions stop by
/// default; ordinary event observers continue by default. Returning a message
/// controls the reply only, rather than secretly changing event flow.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub(crate) propagation: Propagation,
    pub(crate) replies: Vec<Vec<MessageSegment>>,
}

impl Outcome {
    /// Uses the matched handler's normal propagation policy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            propagation: Propagation::Inherit,
            replies: Vec::new(),
        }
    }

    /// Explicitly allows later matching handlers to run.
    #[must_use]
    pub const fn continue_() -> Self {
        Self {
            propagation: Propagation::Continue,
            replies: Vec::new(),
        }
    }

    /// Explicitly prevents later matching handlers from running.
    #[must_use]
    pub const fn stop() -> Self {
        Self {
            propagation: Propagation::Stop,
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
    pub const fn propagation(&self) -> Propagation {
        self.propagation
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.replies.is_empty()
    }

    #[must_use]
    pub fn replies(&self) -> impl ExactSizeIterator<Item = &[MessageSegment]> {
        self.replies.iter().map(Vec::as_slice)
    }

    /// Appends another outcome's replies. An explicit propagation decision in
    /// `next` replaces the current decision; `Inherit` leaves it unchanged.
    #[must_use]
    pub fn and(mut self, mut next: Self) -> Self {
        if next.propagation != Propagation::Inherit {
            self.propagation = next.propagation;
        }
        self.replies.append(&mut next.replies);
        self
    }

    #[must_use]
    pub const fn with_propagation(mut self, propagation: Propagation) -> Self {
        self.propagation = propagation;
        self
    }

    pub(crate) const fn resolve(mut self, default_stop: bool) -> Self {
        if matches!(self.propagation, Propagation::Inherit) {
            self.propagation = if default_stop {
                Propagation::Stop
            } else {
                Propagation::Continue
            };
        }
        self
    }

    pub(crate) const fn is_stopped(&self) -> bool {
        matches!(self.propagation, Propagation::Stop)
    }
}

/// Converts an ordinary successful handler value into Bot effects.
///
/// Fallible handler functions are supported separately when their future
/// returns `Result<T, E>`, `T` implements `IntoOutcome`, and `E` explicitly
/// converts into [`crate::HandlerError`].
pub trait IntoOutcome {
    fn into_outcome(self) -> Outcome;
}

impl IntoOutcome for Outcome {
    fn into_outcome(self) -> Outcome {
        self
    }
}

impl IntoOutcome for () {
    fn into_outcome(self) -> Outcome {
        Outcome::new()
    }
}

impl IntoOutcome for Infallible {
    fn into_outcome(self) -> Outcome {
        match self {}
    }
}

impl IntoOutcome for Message {
    fn into_outcome(self) -> Outcome {
        Outcome::new().reply(self)
    }
}

impl IntoOutcome for MessageSegment {
    fn into_outcome(self) -> Outcome {
        Message::from(self).into_outcome()
    }
}

impl IntoOutcome for Vec<MessageSegment> {
    fn into_outcome(self) -> Outcome {
        Message::from(self).into_outcome()
    }
}

impl IntoOutcome for String {
    fn into_outcome(self) -> Outcome {
        Message::text(self).into_outcome()
    }
}

impl IntoOutcome for &'static str {
    fn into_outcome(self) -> Outcome {
        Message::text(self).into_outcome()
    }
}

impl IntoOutcome for Cow<'static, str> {
    fn into_outcome(self) -> Outcome {
        Message::text(self.into_owned()).into_outcome()
    }
}

impl<T> IntoOutcome for Option<T>
where
    T: IntoOutcome,
{
    fn into_outcome(self) -> Outcome {
        self.map_or_else(Outcome::new, IntoOutcome::into_outcome)
    }
}
