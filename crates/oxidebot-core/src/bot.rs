use crate::api::CallApiTrait;
use std::sync::Arc;

/// Clone-cheap object for the complete OxideBot bot API.
///
/// Handler contexts and the scheduler use this same object.
pub type BotObject = Arc<dyn CallApiTrait>;
