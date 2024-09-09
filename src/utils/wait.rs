use crate::{manager::BroadcastSender, matcher::Matcher, source::message::MessageSegment};
use anyhow::Result;
use std::{
    fmt::{Debug, Display},
    str::FromStr,
    time::Duration,
};

/// A simple wrapper for bool that can be parsed from string, can be easily used in wait generic
#[derive(Clone, Copy)]
pub struct EasyBool(pub bool);

impl From<EasyBool> for bool {
    fn from(easy_bool: EasyBool) -> Self {
        easy_bool.0
    }
}

impl Display for EasyBool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Debug for EasyBool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for EasyBool {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "y" | "yes" | "t" | "true" | "1" => Ok(EasyBool(true)),
            "f" | "false" | "n" | "no" | "0" => Ok(EasyBool(false)),
            _ => Err(anyhow::anyhow!("Invalid easy bool value")),
        }
    }
}

/// wait for any matcher that satisfies the filter_fn
pub async fn wait<F>(
    broadcast_sender: &BroadcastSender,
    timeout: Duration,
    filter_fn: F,
) -> Result<Matcher>
where
    F: Fn(&Matcher) -> bool,
{
    let mut receiver = broadcast_sender.subscribe();
    tokio::time::timeout(timeout, async {
        while let Ok(matcher) = receiver.recv().await {
            if filter_fn(&matcher) {
                return Ok(matcher);
            }
        }
        Err(anyhow::anyhow!("Wait error: Unreachable"))
    })
    .await
    .map_err(|_| anyhow::anyhow!("Wait timed out"))?
}

/// wait for a matcher that contains a text message and the text can be parsed to T
pub async fn wait_text_generic<T, F>(
    broadcast_sender: &BroadcastSender,
    filter_fn: F,
    timeout: Duration,
    mut invalid_threshold: usize,
    error_message: Option<String>,
) -> Result<(T, Matcher)>
where
    T: FromStr,
    T::Err: std::fmt::Debug,
    F: Fn(&Matcher) -> bool,
{
    let mut receiver = broadcast_sender.subscribe();

    invalid_threshold += 1;
    tokio::time::timeout(timeout, async {
        loop {
            if let Ok(matcher) = receiver.recv().await {
                if filter_fn(&matcher) {
                    if let Some(message) = matcher.try_get_message() {
                        let text = message.get_raw_text();

                        match text.parse::<T>() {
                            Ok(value) => return Ok((value, matcher)),
                            Err(err) => {
                                invalid_threshold -= 1;
                                if invalid_threshold == 0 {
                                    if let Some(error_message) = error_message.clone() {
                                        matcher
                                            .try_send_message(vec![MessageSegment::text(format!(
                                                "{}\nError: {:?}\n\nMax retries exceeded, exited.",
                                                error_message, err
                                            ))])
                                            .await?;
                                    }
                                    return Err(anyhow::anyhow!("Max retries exceeded"));
                                } else {
                                    if let Some(error_message) = error_message.clone() {
                                        matcher
                                            .try_send_message(vec![MessageSegment::text(format!(
                                                "{error_message}\nError: {:?}",
                                                err
                                            ))])
                                            .await?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Wait timed out"))?
}

/// wait for a matcher that from the same server and user of the init_matcher
pub async fn wait_user(
    init_matcher: &Matcher,
    broadcast_sender: &BroadcastSender,
    timeout: Duration,
) -> Result<Matcher> {
    let server: &'static str = init_matcher.bot.server();
    let user_id = init_matcher
        .try_get_user()
        .ok_or(anyhow::anyhow!("No user found in init_matcher"))?
        .id
        .clone();

    wait(broadcast_sender, timeout, move |matcher| {
        if server != matcher.bot.server() {
            return false;
        }

        if let Some(user) = matcher.try_get_user() {
            if user.id == user_id {
                return true;
            }
        }
        false
    })
    .await
}

/// wait for a matcher that contains a message from the same server and user of the init_matcher, you can safely unwarp `try_get_message`.
pub async fn wait_user_message(
    init_matcher: &Matcher,
    broadcast_sender: &BroadcastSender,
    timeout: Duration,
) -> Result<Matcher> {
    let server: &'static str = init_matcher.bot.server();
    let user_id = init_matcher
        .try_get_user()
        .ok_or(anyhow::anyhow!("No user found in init_matcher"))?
        .id
        .clone();

    wait(broadcast_sender, timeout, move |matcher| {
        if server != matcher.bot.server() {
            return false;
        }

        if let Some(user) = matcher.try_get_user() {
            if user.id == user_id {
                return matcher.try_get_message().is_some();
            }
        }
        false
    })
    .await
}

/// wait for a matcher that contains a text message and the text can be parsed to T from the same server and user of the init_matcher
pub async fn wait_user_text_generic<T>(
    init_matcher: &Matcher,
    broadcast_sender: &BroadcastSender,
    timeout: Duration,
    invalid_threshold: usize,
    error_message: Option<String>,
) -> Result<(T, Matcher)>
where
    T: FromStr,
    T::Err: std::fmt::Debug,
{
    let server: &'static str = init_matcher.bot.server();
    let user_id = init_matcher
        .try_get_user()
        .ok_or(anyhow::anyhow!("No user found in init_matcher"))?
        .id
        .clone();

    wait_text_generic(
        broadcast_sender,
        move |matcher| {
            if server != matcher.bot.server() {
                return false;
            }

            if let Some(user) = matcher.try_get_user() {
                if user.id == user_id {
                    return true;
                }
            }
            false
        },
        timeout,
        invalid_threshold,
        error_message,
    )
    .await
}
