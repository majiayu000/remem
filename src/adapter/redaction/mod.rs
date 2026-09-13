//! Shared redaction for hook capture previews and MCP/CLI projections.

use crate::db;

mod keys;
mod tokens;

use keys::{is_sensitive_key, looks_like_filesystem_path};
use tokens::{
    contains_inline_sensitive_assignment, redact_inline_sensitive_assignments, redact_tokens,
    split_sensitive_assignment, tokens_contain_sensitive_match,
};

pub(crate) fn redact_token(token: &str) -> String {
    keys::redact_token(token)
}

pub(crate) const HOOK_PAYLOAD_PREVIEW_REDACTION_LOOKAHEAD_BYTES: usize = 4 * 1024;

pub(crate) fn redact_and_truncate(text: &str, max_bytes: usize) -> String {
    let redacted = redact_sensitive_text(text);
    db::truncate_str(&redacted, max_bytes).to_string()
}

pub(crate) fn redact_hook_payload_preview(raw_payload: &str, max_bytes: usize) -> String {
    let preview_input = hook_payload_preview_redaction_input(raw_payload, max_bytes);
    let redacted = serde_json::from_str::<serde_json::Value>(preview_input)
        .map(|value| redact_hook_payload_value(&value).to_string())
        .unwrap_or_else(|_| redact_hook_payload_text(preview_input));
    db::truncate_str(&redacted, max_bytes).to_string()
}

#[cfg(test)]
pub(crate) fn hook_payload_preview_contains_sensitive_match(
    raw_payload: &str,
    max_bytes: usize,
) -> bool {
    let preview_input = hook_payload_preview_redaction_input(raw_payload, max_bytes);
    serde_json::from_str::<serde_json::Value>(preview_input)
        .map(|value| hook_payload_value_contains_sensitive_match(&value))
        .unwrap_or_else(|_| hook_payload_text_contains_sensitive_match(preview_input))
}

pub(crate) fn hook_payload_preview_redaction_input(raw_payload: &str, max_bytes: usize) -> &str {
    db::truncate_str(
        raw_payload,
        max_bytes.saturating_add(HOOK_PAYLOAD_PREVIEW_REDACTION_LOOKAHEAD_BYTES),
    )
}

fn redact_hook_payload_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_key(key) {
                        serde_json::Value::String("[REDACTED]".to_string())
                    } else {
                        redact_hook_payload_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(redact_hook_payload_value)
                .collect::<Vec<_>>(),
        ),
        serde_json::Value::String(text) => {
            serde_json::Value::String(redact_hook_payload_text(text))
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
fn hook_payload_value_contains_sensitive_match(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            is_sensitive_key(key) || hook_payload_value_contains_sensitive_match(value)
        }),
        serde_json::Value::Array(items) => items
            .iter()
            .any(hook_payload_value_contains_sensitive_match),
        serde_json::Value::String(text) => hook_payload_text_contains_sensitive_match(text),
        _ => false,
    }
}

pub(crate) fn redact_sensitive_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_key(key) {
                        serde_json::Value::String("[REDACTED]".to_string())
                    } else {
                        redact_sensitive_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_sensitive_value).collect::<Vec<_>>())
        }
        serde_json::Value::String(text) => serde_json::Value::String(redact_sensitive_text(text)),
        _ => value.clone(),
    }
}

pub(crate) fn redact_sensitive_text(text: &str) -> String {
    text.lines()
        .map(redact_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Redact user-facing projection text (MCP/CLI) including short inline
/// credential assignments such as `Investigate token=abc123` and space-separated
/// sensitive option arguments such as `curl --oauth2-bearer tiny-token`.
///
/// Keep this separate from [`redact_sensitive_text`], which intentionally omits
/// the hook inline-assignment and sensitive-option heuristics to avoid scrubbing
/// ordinary code/prose.
///
/// Filesystem-shaped high-entropy tokens are redacted here. Project identifiers
/// that must stay readable go through [`redact_projected_project_text`].
pub(crate) fn redact_projected_sensitive_text(text: &str) -> String {
    redact_projected_sensitive_text_with_options(text, false)
}

/// Redact projected project identifiers while preserving benign filesystem paths.
pub(crate) fn redact_projected_project_text(text: &str) -> String {
    redact_projected_sensitive_text_with_options(text, true)
}

fn redact_projected_sensitive_text_with_options(
    text: &str,
    preserve_filesystem_paths: bool,
) -> String {
    let redacted = redact_inline_sensitive_assignments(text);
    redacted
        .lines()
        .map(|line| redact_projected_sensitive_line(line, preserve_filesystem_paths))
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_projected_sensitive_line(line: &str, preserve_filesystem_paths: bool) -> String {
    if let Some((prefix, value)) = split_sensitive_assignment(line) {
        return redact_assignment_keeping_suffix(prefix, value);
    }
    // Project identifiers may contain spaces (`/home/u/My Project…`). Classify
    // the complete projected value before whitespace tokenization so long path
    // components are not independently entropy-redacted.
    if preserve_filesystem_paths && looks_like_filesystem_path(line) {
        return line.to_string();
    }
    redact_tokens(line, true, preserve_filesystem_paths)
}

fn redact_sensitive_line(line: &str) -> String {
    if let Some((prefix, _)) = split_sensitive_assignment(line) {
        return format!("{prefix}[REDACTED]");
    }
    redact_tokens(line, false, false)
}

fn redact_hook_payload_text(text: &str) -> String {
    let redacted = redact_inline_sensitive_assignments(text);
    redacted
        .lines()
        .map(redact_hook_payload_line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
fn hook_payload_text_contains_sensitive_match(text: &str) -> bool {
    contains_inline_sensitive_assignment(text)
        || text.lines().any(hook_payload_line_contains_sensitive_match)
}

fn redact_hook_payload_line(line: &str) -> String {
    if let Some((prefix, value)) = split_sensitive_assignment(line) {
        return redact_assignment_keeping_suffix(prefix, value);
    }
    redact_tokens(line, true, false)
}

#[cfg(test)]
fn hook_payload_line_contains_sensitive_match(line: &str) -> bool {
    split_sensitive_assignment(line).is_some() || tokens_contain_sensitive_match(line, true, true)
}

/// Scan procedure-export fields for secrets without the attached short-option
/// heuristic, so benign documentation like `docs mention -username` does not
/// reject an entire export.
pub(crate) fn export_field_contains_sensitive_match(value: &str) -> bool {
    contains_inline_sensitive_assignment(value)
        || value.lines().any(|line| {
            split_sensitive_assignment(line).is_some()
                || tokens_contain_sensitive_match(line, true, false)
        })
}

/// After the bounded inline pass, a line may look like
/// `token=[REDACTED] investigate database regression` or
/// `token="[REDACTED]" investigate database regression`. Preserve the benign
/// suffix instead of treating the whole remainder as the credential value.
fn redact_assignment_keeping_suffix(prefix: &str, value: &str) -> String {
    let trimmed = value.trim_start();
    if let Some(rest) = strip_redacted_assignment_marker(trimmed) {
        return format!("{prefix}[REDACTED]{rest}");
    }
    format!("{prefix}[REDACTED]")
}

fn strip_redacted_assignment_marker(value: &str) -> Option<&str> {
    if let Some(rest) = value.strip_prefix("[REDACTED]") {
        // Only treat whitespace-delimited remainders as benign suffixes. A glued
        // remainder such as `[REDACTED]"abc123"` is leftover secret material.
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return Some(rest);
        }
        return None;
    }
    for quote in ['"', '\''] {
        let mut chars = value.chars();
        if chars.next() != Some(quote) {
            continue;
        }
        let after_open = chars.as_str();
        let Some(after_marker) = after_open.strip_prefix("[REDACTED]") else {
            continue;
        };
        let mut after_marker_chars = after_marker.chars();
        if after_marker_chars.next() != Some(quote) {
            continue;
        }
        let rest = after_marker_chars.as_str();
        // `"abc"tail` is one shell word; a glued remainder is still secret.
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return Some(rest);
        }
        return None;
    }
    None
}
