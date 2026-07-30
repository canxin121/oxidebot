//! Capability-aware message delivery planning.
//!
//! The planner transforms canonical messages into physical delivery plans
//! without I/O. It centralizes fallback, capability adaptation, degradation
//! accounting, and deterministic limit splitting.

use super::*;

#[path = "message_delivery_adapt.rs"]
mod adapt;

#[path = "message_delivery_split.rs"]
mod split;

#[path = "message_delivery_render.rs"]
mod render;

pub(super) use adapt::{adapt_options, adapt_segment, split_message_options};
pub(super) use render::{render_checklist, render_components, render_poll};
pub(super) use split::{split_for_media_limit, split_for_text_limit};
