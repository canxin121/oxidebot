use std::any::Any;
use std::sync::LazyLock;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;

use crate::{api::CallApiTrait, matcher::Matcher, source::bot::BotInfo};

pub type BotObject = Box<dyn BotTrait>;

/// TraitObject can't take self:Arc<Self>, so you should impl Send and Sync And Clone for you bot
/// Tip: use `Arc` to wrap the your bot.
#[async_trait::async_trait]
pub trait BotTrait: Send + Sync + CallApiTrait {
    async fn bot_info(&self) -> BotInfo;
    async fn start_sending_events(&self, sender: broadcast::Sender<Matcher>);
    fn server(&self) -> &'static str;
    // TraitObject can't inherit Clone, so you should manually implement it
    fn clone_box(&self) -> BotObject;
    // TraitObject can't downcast to the concrete type, so you should implement it manually
    fn as_any(&self) -> &dyn Any;
}

impl Clone for BotObject {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for BotObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BotObejct")
    }
}

static GLOBAL_BOTS: LazyLock<RwLock<Vec<BotObject>>> = LazyLock::new(|| RwLock::new(Vec::new()));
static GLOBAL_HANDLERS: LazyLock<RwLock<Vec<JoinHandle<()>>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

pub async fn add_bots(bots: Vec<BotObject>, sender: broadcast::Sender<Matcher>) {
    let mut bots_lock = GLOBAL_BOTS.write().await;
    let mut handlers_lock = GLOBAL_HANDLERS.write().await;

    for bot in bots {
        let bot_ = bot.clone();
        let sender = sender.clone();
        let handler = tokio::spawn(async move {
            bot_.start_sending_events(sender.clone()).await;
        });

        bots_lock.push(bot);
        handlers_lock.push(handler);
    }
}

pub async fn get_bot(server: &str, bot_id: &str) -> Option<BotObject> {
    let bots = GLOBAL_BOTS.read().await; // 使用异步的 .read().await
    for bot in bots.iter() {
        if server != bot.server() {
            continue;
        }
        if let Some(bot_info) = bot.bot_info().await.id {
            if bot_info == bot_id {
                return Some(bot.clone());
            }
        }
    }
    None
}
