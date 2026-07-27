//! Platform-neutral data model shared by OxideBot runtimes and adapters.

mod event;
mod id;
mod message;

pub use event::*;
pub use id::*;
pub use message::*;
