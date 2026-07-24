//! Full Cursor `stop` event validation (GH-823 SP823-T5, consumed by GH-825).
//!
//! Validates the normalized Cursor Stop invocation before any database open,
//! transcript read, enqueue, spill, or LLM call:
//! - exact `hook_event_name == "stop"`;
//! - canonical session identity (`session_id` with required, equal
//!   `conversation_id`) and single normalized workspace root;
//! - `status` restricted to the observed set `{completed, aborted}`
//!   (PR #914 evidence); `error` stays unobserved and fails closed;
//! - non-empty string `generation_id`;
//! - `loop_count` normalized from an exact non-negative integer JSON number
//!   (observed value: `0`); missing, `null`, negative, fractional, or
//!   non-number forms fail closed instead of being guessed as `0`.
//!
//! The canonical Stop key is `(session_id, generation_id, loop_count)`.

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::correlation_id;
use super::identity::{
    field_error, required_non_empty_string, validate_identity_with_required_conversation,
    validate_workspace_root,
};
use super::input::{parse_outer_object, require_event_name};

/// Human-approved accepted Stop status set (PR #914 real-host evidence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStopStatus {
    Completed,
    Aborted,
}

impl CursorStopStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CursorStopStatus::Completed => "completed",
            CursorStopStatus::Aborted => "aborted",
        }
    }
}

/// Sanitized, fully validated Cursor `stop` event.
#[derive(Debug, Clone)]
pub struct CursorStopEvent {
    pub session_id: String,
    /// Normalized sole workspace root; becomes the invocation cwd/project.
    pub workspace_root: String,
    pub status: CursorStopStatus,
    pub generation_id: String,
    /// Normalized non-negative integer loop count.
    pub loop_count: u64,
    /// Null-tolerant base field. `None` for missing/`null`; a string is kept
    /// verbatim (including whitespace-only strings, which downstream maps to
    /// an explicit `path_blank` degradation instead of dropping the Stop).
    pub transcript_path: Option<String>,
}

impl CursorStopEvent {
    /// Canonical idempotency key `(session_id, generation_id, loop_count)`.
    pub fn canonical_stop_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.session_id, self.generation_id, self.loop_count
        )
    }
}

/// Parses and fully validates a Cursor `stop` payload.
pub fn parse_stop_event(bytes: &[u8]) -> Result<CursorStopEvent> {
    let object = parse_outer_object(bytes)?;
    require_event_name(&object, "stop")?;
    let session_id = validate_identity_with_required_conversation(&object)?;
    let workspace_root = validate_workspace_root(&object)?;
    let status = validate_stop_status(&object)?;
    let generation_id = required_non_empty_string(&object, "generation_id")?;
    let loop_count = validate_loop_count(&object)?;
    let transcript_path = stop_transcript_path_field(&object)?;
    Ok(CursorStopEvent {
        session_id,
        workspace_root,
        status,
        generation_id,
        loop_count,
        transcript_path,
    })
}

fn validate_stop_status(object: &serde_json::Map<String, Value>) -> Result<CursorStopStatus> {
    let status = required_non_empty_string(object, "status")?;
    match status.as_str() {
        "completed" => Ok(CursorStopStatus::Completed),
        "aborted" => Ok(CursorStopStatus::Aborted),
        other => Err(anyhow!(
            "cursor stop status '{other}' is outside the approved set \
             (completed, aborted); unobserved statuses fail closed with zero \
             writes [correlation_id={}]",
            correlation_id()
        )),
    }
}

fn validate_loop_count(object: &serde_json::Map<String, Value>) -> Result<u64> {
    match object.get("loop_count") {
        None => Err(field_error("loop_count", "missing")),
        Some(Value::Null) => Err(field_error("loop_count", "null")),
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| field_error("loop_count", "not a non-negative integer")),
        Some(_) => Err(field_error("loop_count", "wrong type")),
    }
}

/// Stop-specific `transcript_path` extraction. Unlike the sessionStart /
/// observe boundary, a whitespace-only string is preserved here so the Stop
/// itself is never lost: the transcript layer degrades it to an explicit
/// `path_blank` reason (GH-825 transcript failure matrix).
fn stop_transcript_path_field(object: &serde_json::Map<String, Value>) -> Result<Option<String>> {
    match object.get("transcript_path") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(path)) => Ok(Some(path.clone())),
        Some(_) => Err(field_error("transcript_path", "wrong type")),
    }
}
