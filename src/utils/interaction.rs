use crate::{matcher::Matcher, source::message::MessageSegment};
use anyhow::Result;
use async_trait::async_trait;
use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};
use tokio::{
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

#[macro_export]
macro_rules! wait_for_input {
    ($receiver:expr, $timeout:expr) => {
        oxidebot::utils::interaction::wait_for_input($receiver, $timeout).await
    };
    ($receiver:expr) => {
        oxidebot::utils::interaction::wait_for_input(
            $receiver,
            std::time::Duration::from_secs(60),
        )
        .await
    };
}

#[macro_export]
macro_rules! wait_for_input_generic {
    ($receiver:expr, $type:ty, $error_message:expr, $max_retries:expr, $timeout:expr) => {
        oxidebot::utils::interaction::wait_for_input_generic::<$type>(
            $receiver,
            Some($error_message.into()),
            $timeout,
            $max_retries,
        )
        .await
    };
    ($receiver:expr, $type:ty, $error_message:expr, $max_retries:expr) => {
        oxidebot::utils::interaction::wait_for_input_generic::<$type>(
            $receiver,
            Some($error_message.into()),
            std::time::Duration::from_secs(60),
            $max_retries,
        )
        .await
    };
    ($receiver:expr, $type:ty, $error_message:expr) => {
        oxidebot::utils::interaction::wait_for_input_generic::<$type>(
            $receiver,
            Some($error_message.into()),
            std::time::Duration::from_secs(60),
            3,
        )
        .await
    };
    ($receiver:expr, $type:ty) => {
        oxidebot::utils::interaction::wait_for_input_generic::<$type>(
            $receiver,
            None,
            std::time::Duration::from_secs(60),
            0,
        )
        .await
    };
    ($receiver:expr) => {
        oxidebot::utils::interaction::wait_for_input_generic::<String>(
            $receiver,
            None,
            std::time::Duration::from_secs(60),
            0,
        )
        .await
    };
}

pub async fn wait_for_input(
    receiver: &mut Receiver<Matcher>,
    timeout: Duration,
) -> Result<Matcher> {
    tokio::time::timeout(timeout, async {
        while let Some(matcher) = receiver.recv().await {
            return Ok(matcher);
        }
        Err(anyhow::anyhow!("No message received"))
    })
    .await?
}

pub async fn wait_for_input_generic<T>(
    receiver: &mut Receiver<Matcher>,
    error_message: Option<String>,
    timeout: Duration,
    max_retries: usize,
) -> Result<(T, Matcher)>
where
    T: FromStr,
    T::Err: std::fmt::Debug,
{
    let mut attempts = 0;

    tokio::time::timeout(timeout, async {
        loop {
            if let Some(matcher) = receiver.recv().await {
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

                            if attempts > max_retries {
                                return Err(anyhow::anyhow!("Max retries exceeded"));
                            }
                        }
                    }
                }
            }
        }
    })
    .await?
}

#[async_trait]
pub trait InteractionTrait: Send + Sync + 'static {
    async fn should_start(&self, matcher: Matcher) -> bool;
    async fn handle_interaction<'a>(
        &'a self,
        mut init_matcher: Matcher,
        mut receiver: Receiver<Matcher>,
    ) -> Result<()>;
}

pub struct Interaction<T: InteractionTrait> {
    pub ids: Mutex<HashMap<String, Sender<Matcher>>>,
    interaction: T,
}

impl<T: InteractionTrait> Interaction<T> {
    pub fn new(interaction: T) -> Arc<Self> {
        Arc::new(Self {
            ids: Mutex::new(HashMap::new()),
            interaction,
        })
    }

    // use this method in you EventHandler to easily interact with the user
    pub fn interact(self: Arc<Self>, matcher: &Matcher) -> Result<()> {
        let matcher = matcher.clone();
        let self_clone = self.clone();
        let _: JoinHandle<Result<()>> = tokio::spawn(async move {
            if let Some(id) = matcher.try_get_user().map(|u| u.id.to_string()) {
                if self_clone.interaction.should_start(matcher.clone()).await {
                    self_clone.clone().add_id(id, matcher.clone()).await;
                } else {
                    self_clone.try_send_to_user(&id, matcher).await?;
                }
            }
            Ok(())
        });
        Ok(())
    }

    async fn add_id(self: Arc<Self>, id: String, matcher: Matcher) {
        let mut ids = self.ids.lock().await;
        if !ids.contains_key(&id) {
            let (sender, receiver) = mpsc::channel(8);
            ids.insert(id.clone(), sender);
            let self_clone = self.clone();
            tokio::spawn(async move {
                if let Err(e) = self_clone
                    .interaction
                    .handle_interaction(matcher, receiver)
                    .await
                {
                    tracing::error!("Error in interaction: {}", e);
                }
                self_clone.remove_id(&id).await
            });
        }
    }

    async fn try_send_to_user(&self, id: &str, matcher: Matcher) -> Result<()> {
        let ids = self.ids.lock().await;
        if let Some(sender) = ids.get(id) {
            sender
                .send(matcher)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        Ok(())
    }

    async fn remove_id(&self, id: &str) {
        let mut ids = self.ids.lock().await;
        if let Some(sender) = ids.remove(id) {
            drop(sender);
        }
    }
}
