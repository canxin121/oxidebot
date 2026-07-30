//! Structure-preserving message templates and typed segment selectors.

#[path = "template_catalog.rs"]
mod catalog;
#[path = "template_engine.rs"]
mod engine;

pub use catalog::*;
pub use engine::*;
