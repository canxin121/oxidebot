//! Deterministic platform fixtures for runtime integration tests.

pub mod adapter_contract;
mod command;
mod frame;
mod scenario;
mod scripted;

pub use command::{command_test, command_tree_test, CommandTest};
pub use frame::{DecodeCounter, InteractionFrame, TestFrame};
pub use scenario::{BotTest, BotTestReport, InteractionExpectation, ReplyExpectation};
pub use scripted::{InteractionCall, ScriptStep, ScriptedAdapter, ScriptedApi, SentMessage};
