//! Runtime orchestration, compiled dispatch, bounded queues, and adapter contracts.

mod adapter;
mod address;
mod app;
mod authoring;
mod bot;
mod budget;
mod cancellation;
mod command;
mod config;
mod context;
mod dedupe;
mod dialogue;
mod error;
mod executor;
mod extract;
mod filter;
mod function;
mod handler;
mod hooks;
mod media;
mod messenger;
mod metrics;
mod module;
mod outcome;
mod plugin;
mod reply;
mod responder;
mod router;
mod service;
mod session;
mod standard;

pub use adapter::*;
pub use address::*;
pub use app::*;
pub use authoring::*;
pub use bot::*;
pub use budget::{OverloadPolicy, QueueBudget};
pub use cancellation::ShutdownSignal;
pub use command::*;
pub use config::*;
pub use context::Context;
pub use dialogue::*;
pub use error::*;
pub use extract::*;
pub use filter::*;
#[doc(hidden)]
pub use function::{FallibleOutput, InfallibleOutput, IntoHandler};
pub use handler::{EventContext, MessageContext};
pub use hooks::{After, Before, Guard, GuardDecision, GuardResult};
#[doc(hidden)]
pub use hooks::{AfterOutput, BeforeOutput, Endpoint, GuardOutput};
pub use media::*;
pub use messenger::*;
pub use metrics::*;
pub use module::{Feature, FeatureExt, GeneratedFeature, IntoFeature, Module};
pub use outcome::{IntoOutcome, Outcome, Propagation};
pub use plugin::{PluginBundle, PluginMetadata, PluginRequirement};
pub use reply::*;
pub use responder::*;
pub use service::*;
pub use session::{AskOptions, SessionEvent, SessionKey, SessionPolicy, SessionRegistry};
pub use standard::*;

/// Implementation details used by the facade macros.
///
/// This module is deliberately hidden from ordinary API documentation. It
/// gives proc-macro expansions a stable path without forcing applications to
/// depend on `async-trait` directly.
#[doc(hidden)]
pub mod __private {
    pub use async_trait::async_trait;
}
