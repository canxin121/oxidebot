use crate::{manager::BroadcastSender, matcher::Matcher, source::message::MessageSegment};
use anyhow::Result;
use std::{str::FromStr, time::Duration};

/// wait for any matcher that satisfies the filter_fn
/// timeout: seconds
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
        Err(anyhow::anyhow!("Wait Timeout"))
    })
    .await?
}

/// wait for a matcher that contains a text message and the text can be parsed to T
/// timeout: seconds
pub async fn wait_text_generic<T, F>(
    broadcast_sender: &BroadcastSender,
    filter_fn: F,
    timeout: Duration,
    invalid_threshold: usize,
    error_message: Option<String>,
) -> Result<(T, Matcher)>
where
    T: FromStr,
    T::Err: std::fmt::Debug,
    F: Fn(&Matcher) -> bool,
{
    let mut receiver = broadcast_sender.subscribe();
    let mut attempts = 0;

    tokio::time::timeout(timeout, async {
        loop {
            if let Ok(matcher) = receiver.recv().await {
                if filter_fn(&matcher) {
                    if let Some(message) = matcher.try_get_message() {
                        let text = message.get_raw_text();

                        match text.parse::<T>() {
                            Ok(value) => return Ok((value, matcher)),
                            Err(e) => {
                                attempts += 1;

                                if let Some(error_message) = error_message.clone() {
                                    matcher
                                        .try_send_message(vec![MessageSegment::text(error_message)])
                                        .await?;
                                }

                                tracing::error!(
                                    "Interaction Error: {:?}, Expected type: {}, got: {}",
                                    e,
                                    std::any::type_name::<T>(),
                                    text
                                );

                                if attempts > invalid_threshold {
                                    return Err(anyhow::anyhow!("Max retries exceeded"));
                                }
                            }
                        }
                    }
                }
            }
        }
    })
    .await?
}

/// wait for a matcher that from the same server and user of the init_matcher
/// timeout: seconds
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
/// timeout: seconds
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
/// timeout: seconds
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
