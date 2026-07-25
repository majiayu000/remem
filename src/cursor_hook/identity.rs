//! Cursor event identity and workspace-root validation (B-002, B-013).
//!
//! Every helper takes the already-parsed outer JSON object and returns
//! fail-closed errors whose messages contain only field names/types and a
//! correlation id, never raw payload content.

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::correlation_id;

/// Validated canonical event identity: `session_id` with event-local
/// `session_id == conversation_id` equality (when both are present).
/// Subagent events carry their own internally-equal identity and are accepted
/// as distinct sessions; no parent-child coercion happens here.
pub fn validate_identity(object: &serde_json::Map<String, Value>) -> Result<String> {
    let session_id = required_non_empty_string(object, "session_id")?;
    match object.get("conversation_id") {
        None => {}
        Some(Value::String(conversation_id)) => {
            if conversation_id.trim().is_empty() {
                return Err(field_error("conversation_id", "empty string"));
            }
            if conversation_id != &session_id {
                return Err(anyhow!(
                    "cursor hook payload violates event-local identity equality \
                     (session_id != conversation_id) [correlation_id={}]",
                    correlation_id()
                ));
            }
        }
        Some(_) => return Err(field_error("conversation_id", "wrong type")),
    }
    Ok(session_id)
}

/// Like [`validate_identity`] but requires `conversation_id` to be present
/// (observe/stop contract: the required non-empty `conversation_id` maps to
/// the canonical `session_id`).
pub fn validate_identity_with_required_conversation(
    object: &serde_json::Map<String, Value>,
) -> Result<String> {
    if !object.contains_key("conversation_id") {
        return Err(field_error("conversation_id", "missing"));
    }
    validate_identity(object)
}

/// Validates `workspace_roots` as an array whose total length is exactly one
/// and whose sole string is non-empty after trimming, then normalizes it via
/// the platform-aware normalizer. `[]`, `[""]`, mixed blank arrays, and
/// multi-root arrays all fail closed; blank entries are never filtered before
/// the length check (B-013).
pub fn validate_workspace_root(object: &serde_json::Map<String, Value>) -> Result<String> {
    let Some(value) = object.get("workspace_roots") else {
        return Err(field_error("workspace_roots", "missing"));
    };
    let Value::Array(roots) = value else {
        return Err(field_error("workspace_roots", "wrong type"));
    };
    if roots.len() != 1 {
        return Err(anyhow!(
            "cursor hook payload field 'workspace_roots' must contain exactly one root, \
             got {} [correlation_id={}]",
            roots.len(),
            correlation_id()
        ));
    }
    let Value::String(root) = &roots[0] else {
        return Err(field_error("workspace_roots[0]", "wrong type"));
    };
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return Err(field_error("workspace_roots[0]", "empty string"));
    }
    normalize_workspace_root(trimmed)
}

/// Platform-aware workspace-root normalizer. Only shapes backed by sanitized
/// #822/PR #914 real-host evidence are accepted: absolute Unix paths observed
/// on macOS. Windows drive forms (`/c:/...`, `C:\...`), UNC paths, and
/// relative paths are unverified and fail closed; the raw string is never
/// persisted as project identity (B-002, R5).
pub fn normalize_workspace_root(trimmed: &str) -> Result<String> {
    let unverified = |shape: &str| {
        anyhow!(
            "cursor workspace root has an unverified platform shape ({shape}); \
             failing closed without persisting raw path identity [correlation_id={}]",
            correlation_id()
        )
    };
    if trimmed.contains('\\') {
        return Err(unverified("backslash path"));
    }
    if !trimmed.starts_with('/') {
        return Err(unverified("relative or drive-letter path"));
    }
    if trimmed.starts_with("//") {
        return Err(unverified("UNC-like path"));
    }
    let mut chars = trimmed.chars();
    let _slash = chars.next();
    if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() && second == ':' {
            return Err(unverified("windows drive path"));
        }
    }
    Ok(trimmed.to_string())
}

/// Validates the null-tolerant `transcript_path` base field: missing or
/// `null` maps to `None`, a non-empty string maps to `Some`. Empty strings
/// and other types fail closed. A null child path is never replaced with a
/// parent path here or anywhere downstream.
pub fn validate_transcript_path(object: &serde_json::Map<String, Value>) -> Result<Option<String>> {
    match object.get("transcript_path") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(path)) => {
            if path.trim().is_empty() {
                Err(field_error("transcript_path", "empty string"))
            } else {
                Ok(Some(path.clone()))
            }
        }
        Some(_) => Err(field_error("transcript_path", "wrong type")),
    }
}

pub(super) fn required_non_empty_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<String> {
    match object.get(field) {
        None => Err(field_error(field, "missing")),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                Err(field_error(field, "empty string"))
            } else {
                Ok(value.clone())
            }
        }
        Some(_) => Err(field_error(field, "wrong type")),
    }
}

pub(super) fn field_error(field: &str, problem: &str) -> anyhow::Error {
    anyhow!(
        "cursor hook payload field '{field}' invalid: {problem} [correlation_id={}]",
        correlation_id()
    )
}
