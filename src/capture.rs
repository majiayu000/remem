//! Capture-path entry point. Wraps the conversion from a host-supplied
//! hook event into a `captured_events` row, resolving the foreign keys for
//! host / workspace / project / session along the way. Hook adapters and
//! the capture command consume this; nothing else should INSERT into
//! `captured_events` directly.

mod blob;
mod insert;
mod types;

pub use insert::{ensure_session, insert_captured_event};
pub use types::NormalizedEvent;
