#![doc = include_str!("/mnt/disk/git/oxidebot/Readme.md")]

pub mod api;
pub mod bot;
pub mod event;
pub mod filter;
pub mod handler;
pub mod manager;
pub mod matcher;
pub mod source;
pub mod utils;

pub use api::CallApiTrait;
pub use bot::{get_bot, BotTrait};
pub use event::EventTrait;
pub use filter::FilterTrait;
pub use handler::ActiveHandlerTrait;
pub use handler::EventHandlerTrait;
pub use handler::Handler;
pub use manager::OxideBotManager;

pub use utils::wait::{
    wait, wait_text_generic, wait_user, wait_user_message, wait_user_text_generic, EasyBool,
};
