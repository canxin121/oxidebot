use crate::{api::CallApiTrait, source::bot::BotInfo};
use std::sync::Arc;

/// Clone-cheap object for the complete OxideBot bot API.
///
/// Event convenience methods and handler contexts use this same object, so an
/// adapter never has to expose a second runtime-specific API handle.
pub type BotObject = Arc<dyn CallApiTrait>;

/// Identity-bearing bot abstraction. Runtime adapters normally provide identity
/// through `BotDescriptor` and the API implementation through `BotServices::new`.
#[async_trait::async_trait]
pub trait BotTrait: Send + Sync + CallApiTrait + 'static {
    async fn bot_info(&self) -> BotInfo;
    fn server(&self) -> &'static str;
}
