//! Bounded Cursor stdin reading and fail-closed payload parsing
//! (B-002, B-003, B-007, B-009, B-014, B-015).
//!
//! Parsing copies only whitelisted fields out of the outer JSON object and
//! then drops it: `user_email` and every other non-canonical field never
//! reach a canonical event, log line, error message, or preview (B-014).

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::io::Read;

use super::identity::{
    field_error, required_non_empty_string, validate_identity,
    validate_identity_with_required_conversation, validate_transcript_path,
    validate_workspace_root,
};
use super::{correlation_id, CURSOR_HOOK_STDIN_MAX_BYTES, CURSOR_TOOL_FIELD_MAX_BYTES};

/// Sanitized, validated Cursor `sessionStart` event for `remem context`.
#[derive(Debug, Clone)]
pub struct CursorSessionStart {
    pub session_id: String,
    /// Normalized sole workspace root; becomes the invocation cwd/project.
    pub workspace_root: String,
    /// Null-tolerant base field; `None` on new sessions.
    pub transcript_path: Option<String>,
}

/// Sanitized, validated Cursor tool event for `remem observe`.
#[derive(Debug, Clone)]
pub struct CursorToolEvent {
    pub session_id: String,
    pub workspace_root: String,
    pub tool_name: String,
    /// Canonical per-call event/upsert identity (SP823-T2 item 5).
    pub tool_use_id: String,
    /// Observed generic `tool_input` object (canonical form is bounded).
    pub tool_input: serde_json::Map<String, Value>,
    pub outcome: CursorToolOutcome,
    pub transcript_path: Option<String>,
}

/// Canonical tool outcome preserved through capture, spill, and replay.
#[derive(Debug, Clone)]
pub enum CursorToolOutcome {
    Success {
        tool_output: String,
    },
    Failure {
        error_message: String,
        failure_type: String,
        duration: f64,
        is_interrupt: bool,
    },
}

impl CursorToolOutcome {
    pub fn is_failure(&self) -> bool {
        matches!(self, CursorToolOutcome::Failure { .. })
    }
}

/// Generic-success tool names whose post-tool variants remain unobserved on
/// Cursor 3.12.17 and are therefore fail-closed by the SP823-T2 approval
/// (item 7): Task completion uses the subagent lifecycle, and Write/Edit/
/// Delete were never exercised by the read-only probe. They are not treated
/// as unknown names; they are explicitly disabled.
const FAIL_CLOSED_SUCCESS_TOOL_NAMES: [&str; 4] = ["Task", "Write", "Edit", "Delete"];

/// The only observed `postToolUseFailure` shape is a failed `Read`
/// (PR #914); no other failure event is accepted by analogy.
const ACCEPTED_FAILURE_TOOL_NAMES: [&str; 1] = ["Read"];

/// Reads at most `CURSOR_HOOK_STDIN_MAX_BYTES + 1` bytes and rejects the
/// one-byte-over sentinel before any UTF-8 conversion, `String` allocation,
/// serde parse, or payload preview (B-009). The error records only the
/// configured bound and a correlation id.
pub fn read_bounded_hook_stdin(reader: &mut dyn Read) -> Result<Vec<u8>> {
    read_bounded_hook_input(reader, CURSOR_HOOK_STDIN_MAX_BYTES)
}

pub fn read_bounded_hook_input(reader: &mut dyn Read, max_bytes: usize) -> Result<Vec<u8>> {
    let mut limited = reader.take(max_bytes as u64 + 1);
    let mut buffer = Vec::new();
    limited
        .read_to_end(&mut buffer)
        .map_err(|_| size_only_error(max_bytes, "stdin read failed"))?;
    if buffer.len() > max_bytes {
        return Err(size_only_error(max_bytes, "stdin exceeds configured bound"));
    }
    Ok(buffer)
}

fn size_only_error(max_bytes: usize, problem: &str) -> anyhow::Error {
    anyhow!(
        "cursor hook stdin rejected: {problem} (limit={max_bytes} bytes) [correlation_id={}]",
        correlation_id()
    )
}

/// Parses and validates a Cursor `sessionStart` payload for `remem context`.
/// Any other event name is an event/command mismatch and fails closed.
pub fn parse_session_start(bytes: &[u8]) -> Result<CursorSessionStart> {
    let object = parse_outer_object(bytes)?;
    require_event_name(&object, "sessionStart")?;
    let session_id = validate_identity(&object)?;
    let workspace_root = validate_workspace_root(&object)?;
    let transcript_path = validate_transcript_path(&object)?;
    Ok(CursorSessionStart {
        session_id,
        workspace_root,
        transcript_path,
    })
}

/// Parses and validates a Cursor observe payload. Only exact `postToolUse`
/// and `postToolUseFailure` are accepted (B-016 generic ownership keeps
/// `beforeMCPExecution`/`afterMCPExecution` unregistered and unsupported).
pub fn parse_observe_event(bytes: &[u8]) -> Result<CursorToolEvent> {
    let object = parse_outer_object(bytes)?;
    let event_name = required_non_empty_string(&object, "hook_event_name")?;
    let outcome = match event_name.as_str() {
        "postToolUse" => parse_success_outcome(&object)?,
        "postToolUseFailure" => parse_failure_outcome(&object)?,
        _ => {
            return Err(anyhow!(
                "cursor observe rejects hook_event_name '{event_name}': only exact \
                 postToolUse and postToolUseFailure are supported (MCP-specific events \
                 stay unregistered under generic ownership) [correlation_id={}]",
                correlation_id()
            ))
        }
    };
    let session_id = validate_identity_with_required_conversation(&object)?;
    let workspace_root = validate_workspace_root(&object)?;
    let transcript_path = validate_transcript_path(&object)?;
    let tool_name = required_non_empty_string(&object, "tool_name")?;
    validate_tool_name_support(&tool_name, &outcome)?;
    let tool_use_id = required_non_empty_string(&object, "tool_use_id")?;
    let tool_input = validate_tool_input(&object)?;
    Ok(CursorToolEvent {
        session_id,
        workspace_root,
        tool_name,
        tool_use_id,
        tool_input,
        outcome,
        transcript_path,
    })
}

/// Validates that a summarize payload is an exact Cursor `stop` event.
/// The transcript path and every other field are dropped at this boundary:
/// Cursor summarize stays fail-closed until GH-825's verified transcript
/// reader lands (SP823-T5), so nothing may flow toward the Claude/Codex
/// reader, enqueue, spill, or LLM paths.
pub fn require_stop_event(bytes: &[u8]) -> Result<()> {
    let object = parse_outer_object(bytes)?;
    require_event_name(&object, "stop")?;
    Ok(())
}

pub(super) fn parse_outer_object(bytes: &[u8]) -> Result<serde_json::Map<String, Value>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        anyhow!(
            "cursor hook payload is not valid UTF-8 [correlation_id={}]",
            correlation_id()
        )
    })?;
    let value: Value = serde_json::from_str(text).map_err(|error| {
        anyhow!(
            "cursor hook payload is not valid JSON (line {}, column {}) [correlation_id={}]",
            error.line(),
            error.column(),
            correlation_id()
        )
    })?;
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(anyhow!(
            "cursor hook payload is not a JSON object [correlation_id={}]",
            correlation_id()
        )),
    }
}

pub(super) fn require_event_name(
    object: &serde_json::Map<String, Value>,
    expected: &str,
) -> Result<()> {
    let event_name = required_non_empty_string(object, "hook_event_name")?;
    if event_name != expected {
        return Err(anyhow!(
            "cursor hook event/command mismatch: got hook_event_name '{event_name}', \
             this command supports only exact '{expected}' [correlation_id={}]",
            correlation_id()
        ));
    }
    Ok(())
}

fn parse_success_outcome(object: &serde_json::Map<String, Value>) -> Result<CursorToolOutcome> {
    let Some(value) = object.get("tool_output") else {
        return Err(field_error("tool_output", "missing"));
    };
    let Value::String(tool_output) = value else {
        return Err(field_error("tool_output", "wrong type"));
    };
    check_field_bytes("tool_output", tool_output.len())?;
    Ok(CursorToolOutcome::Success {
        tool_output: tool_output.clone(),
    })
}

fn parse_failure_outcome(object: &serde_json::Map<String, Value>) -> Result<CursorToolOutcome> {
    let error_message = required_non_empty_string(object, "error_message")?;
    check_field_bytes("error_message", error_message.len())?;
    let failure_type = required_non_empty_string(object, "failure_type")?;
    if failure_type != "error" {
        return Err(field_error("failure_type", "unobserved value"));
    }
    let duration = match object.get("duration") {
        Some(Value::Number(number)) => number
            .as_f64()
            .ok_or_else(|| field_error("duration", "non-finite number"))?,
        Some(_) => return Err(field_error("duration", "wrong type")),
        None => return Err(field_error("duration", "missing")),
    };
    let is_interrupt = match object.get("is_interrupt") {
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(field_error("is_interrupt", "wrong type")),
        None => return Err(field_error("is_interrupt", "missing")),
    };
    Ok(CursorToolOutcome::Failure {
        error_message,
        failure_type,
        duration,
        is_interrupt,
    })
}

fn validate_tool_name_support(tool_name: &str, outcome: &CursorToolOutcome) -> Result<()> {
    if outcome.is_failure() {
        if !ACCEPTED_FAILURE_TOOL_NAMES.contains(&tool_name) {
            return Err(anyhow!(
                "cursor postToolUseFailure for tool '{tool_name}' is unobserved and \
                 stays fail-closed (only the PR #914 failed Read shape is accepted) \
                 [correlation_id={}]",
                correlation_id()
            ));
        }
        return Ok(());
    }
    if FAIL_CLOSED_SUCCESS_TOOL_NAMES.contains(&tool_name) {
        return Err(anyhow!(
            "cursor postToolUse for tool '{tool_name}' is an unobserved variant and \
             stays fail-closed per the SP823-T2 approval [correlation_id={}]",
            correlation_id()
        ));
    }
    Ok(())
}

fn validate_tool_input(
    object: &serde_json::Map<String, Value>,
) -> Result<serde_json::Map<String, Value>> {
    let Some(value) = object.get("tool_input") else {
        return Err(field_error("tool_input", "missing"));
    };
    let Value::Object(tool_input) = value else {
        return Err(field_error("tool_input", "wrong type"));
    };
    let canonical = serde_json::to_string(tool_input)
        .map_err(|_| field_error("tool_input", "not canonically serializable"))?;
    check_field_bytes("tool_input", canonical.len())?;
    Ok(tool_input.clone())
}

fn check_field_bytes(field: &str, len: usize) -> Result<()> {
    if len > CURSOR_TOOL_FIELD_MAX_BYTES {
        return Err(anyhow!(
            "cursor hook payload field '{field}' exceeds the configured bound \
             (limit={CURSOR_TOOL_FIELD_MAX_BYTES} bytes) [correlation_id={}]",
            correlation_id()
        ));
    }
    Ok(())
}
