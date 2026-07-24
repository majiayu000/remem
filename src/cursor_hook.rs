//! Cursor hook I/O protocol boundary (GH-823).
//!
//! This module owns the shared Cursor stdin/payload contract for the
//! `context`, `observe`, and `summarize` hook commands. Command entrypoints
//! only dispatch and adapt; every Cursor payload is read through the bounded
//! byte reader, sanitized (PII such as `user_email` is dropped at the outer
//! boundary and never copied into canonical events), and validated fail-closed
//! against the payload shapes observed on Cursor 3.12.17 (PR #914 evidence).
//!
//! Frozen decisions (issue #823 SP823-T2 approval record, 2026-07-24):
//! - `CURSOR_HOOK_STDIN_MAX_BYTES = 1_048_576`, independent from
//!   `CURSOR_TOOL_FIELD_MAX_BYTES`.
//! - Cursor 3.12.17 sessionStart injection stays disabled; GH-823 v1 adds no
//!   postToolUse context command/renderer/install entry.
//! - `session_id == conversation_id` equality is required on every event that
//!   carries both; subagent identities stay distinct.
//! - Generic success requires a non-empty string `tool_use_id`; it is the
//!   canonical per-call event/upsert identity.
//! - B-016 MCP ownership is generic: `beforeMCPExecution` and
//!   `afterMCPExecution` are both unregistered/unsupported.
//! - All unobserved paths (Task success, Write/Edit/Delete, failed Shell,
//!   multi-root, Windows/UNC, stop `status:"error"`) stay fail-closed.

pub mod identity;
pub mod input;

#[cfg(test)]
#[path = "cursor_hook/tests.rs"]
mod tests;

/// Whole-payload stdin bound applied before any `String` allocation or serde
/// parse on every Cursor hook entrypoint. Human-frozen (SP823-T2 item 1).
pub const CURSOR_HOOK_STDIN_MAX_BYTES: usize = 1_048_576;

/// Per-field bound for Cursor tool payload representations (raw generic
/// strings measured as exact UTF-8 bytes; object representations measured on
/// their canonical serialization). Independent from
/// `CURSOR_HOOK_STDIN_MAX_BYTES` per the SP823-T2 approval record; the
/// numeric value below is the implementation proposal pending explicit human
/// confirmation on issue #823.
pub const CURSOR_TOOL_FIELD_MAX_BYTES: usize = 262_144;

/// Canonical host value recorded in the database for Cursor-origin events
/// (B-011). Must match `crate::identity::InstallHost::Cursor.as_db_value()`.
pub const CURSOR_HOST: &str = "cursor";

/// Existing `captured_events.event_type` text discriminator for the observed
/// failed-tool path (B-007). No migration, table, column, or index is added.
pub const CURSOR_TOOL_FAILURE_EVENT_TYPE: &str = "cursor_tool_failure";

/// Maps a capture summary event type to the `captured_events.event_type`
/// discriminator: the Cursor failed-tool value is preserved verbatim (B-007,
/// including spill replay); everything else keeps the historical
/// `tool_result` value used by Claude/Codex captures.
pub(crate) fn captured_event_type_for_summary(summary_event_type: &str) -> &'static str {
    if summary_event_type == CURSOR_TOOL_FAILURE_EVENT_TYPE {
        CURSOR_TOOL_FAILURE_EVENT_TYPE
    } else {
        "tool_result"
    }
}

/// Correlation id attached to Cursor parse/limit errors so failures can be
/// traced in logs without ever echoing raw payload bytes.
pub(crate) fn correlation_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("cursor-{:x}-{:x}", std::process::id(), nanos)
}
