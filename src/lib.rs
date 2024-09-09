pub mod api;
pub mod bot;
pub mod event;
pub mod filter;
pub mod handler;
pub mod manager;
pub mod matcher;
pub mod source;
pub mod utils;

pub use bot::{BotObject, BotTrait};
pub use event::{EventObject, EventTrait};
pub use handler::{EventHandlerObject, EventHandlerTrait};
pub use manager::OxideBotManager;
