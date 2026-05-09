//! v2 capture-path entry point. Wraps the conversion from a host-supplied
//! hook event into a `captured_events` row, resolving the foreign keys for
//! host / workspace / project / session along the way. Hook adapters and
//! the v2 capture command consume this; nothing else should INSERT into
//! `captured_events` directly.
//!
//! See SPEC-memory-system-v2-no-compat §6.5 (captured_events) and §7
//! (capture path) plus SPEC-memory-system-v2.1-revisions §4 D1 for the
//! `content_text` storage policy.

mod insert;
mod types;

pub use insert::{ensure_session, insert_captured_event};
pub use types::NormalizedEvent;
