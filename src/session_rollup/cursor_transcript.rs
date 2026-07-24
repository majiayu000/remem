//! Cursor JSONL transcript full validation and canonical IR (GH-825).
//!
//! The parser accepts exactly the record grammar observed on real hosts
//! (PR #874 Cursor 3.6.31 and PR #914 Cursor 3.12.17):
//! - `{role,message}` records whose `message.content` is an array of `text`
//!   blocks and (assistant-only) `tool_use` blocks;
//! - standalone `{type:"turn_ended",status:"success"|"error"}` boundary
//!   records (the `error` form carries the abort/cancel boundary).
//!
//! Validation is all-or-nothing: any signature, encoding, record, or tail
//! error invalidates the whole input; no valid prefix is ever returned.
//! Records without per-message IDs or timestamps are valid — those fields
//! were never observed and are never synthesized. Every record receives a
//! zero-based physical ordinal in stable JSONL line order, assigned before
//! any record-type or role/usability projection, so message ordinals may
//! have holes but can never be reordered or compacted.
//!
//! This parser is Cursor-only. Claude Code and Codex transcripts must never
//! be routed here, and Cursor transcripts must never reach the Claude/Codex
//! raw transcript classifier.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// One fully validated Cursor transcript snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedCursorTranscript {
    /// Hex SHA-256 of the complete stable snapshot bytes.
    pub(crate) snapshot_hash: String,
    pub(crate) snapshot_byte_len: u64,
    pub(crate) records: Vec<CursorTranscriptRecord>,
}

/// Versioned record enum for the observed Cursor JSONL grammar. Every
/// variant carries its zero-based physical record ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CursorTranscriptRecord {
    Message {
        ordinal: u64,
        role: CursorMessageRole,
        /// Concatenated `text` block content in block order, joined with a
        /// newline; whitespace is preserved verbatim.
        text: String,
        /// Names of assistant `tool_use` blocks, in block order. Tool
        /// payloads stay in the capture ledger and never masquerade as
        /// conversation text.
        tool_use_names: Vec<String>,
    },
    /// Internal turn boundary/status evidence. Never enters raw archive or
    /// prompt projections.
    TurnEnded {
        ordinal: u64,
        status: CursorTurnEndedStatus,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorMessageRole {
    User,
    Assistant,
}

impl CursorMessageRole {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            CursorMessageRole::User => "user",
            CursorMessageRole::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorTurnEndedStatus {
    Success,
    Error,
}

impl ValidatedCursorTranscript {
    /// Usable conversation projection: `(ordinal, role, text)` for every
    /// message record whose text is non-empty after trimming. Ordinals keep
    /// their physical values, so holes from `turn_ended` records remain.
    pub(crate) fn usable_messages(&self) -> impl Iterator<Item = (u64, CursorMessageRole, &str)> {
        self.records.iter().filter_map(|record| match record {
            CursorTranscriptRecord::Message {
                ordinal,
                role,
                text,
                ..
            } if !text.trim().is_empty() => Some((*ordinal, *role, text.as_str())),
            _ => None,
        })
    }

    pub(crate) fn last_assistant_text(&self) -> Option<&str> {
        self.records.iter().rev().find_map(|record| match record {
            CursorTranscriptRecord::Message {
                role: CursorMessageRole::Assistant,
                text,
                ..
            } if !text.trim().is_empty() => Some(text.as_str()),
            _ => None,
        })
    }
}

/// Fully validates a complete stable Cursor transcript snapshot.
pub(crate) fn parse_cursor_transcript(bytes: &[u8]) -> Result<ValidatedCursorTranscript> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| anyhow!("cursor transcript is not valid UTF-8"))?;
    let mut records = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let ordinal = u64::try_from(line_index).expect("line index fits u64");
        if line.trim().is_empty() {
            bail!("cursor transcript record {ordinal} is blank (unobserved grammar)");
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            anyhow!(
                "cursor transcript record {ordinal} is not valid JSON (column {})",
                error.column()
            )
        })?;
        let Value::Object(object) = value else {
            bail!("cursor transcript record {ordinal} is not a JSON object");
        };
        records.push(parse_record(ordinal, &object)?);
    }
    Ok(ValidatedCursorTranscript {
        snapshot_hash: format!("{:x}", Sha256::digest(bytes)),
        snapshot_byte_len: bytes.len() as u64,
        records,
    })
}

fn parse_record(
    ordinal: u64,
    object: &serde_json::Map<String, Value>,
) -> Result<CursorTranscriptRecord> {
    if object.contains_key("role") {
        return parse_message_record(ordinal, object);
    }
    if object.get("type").is_some() {
        return parse_turn_ended_record(ordinal, object);
    }
    bail!("cursor transcript record {ordinal} matches no observed record shape")
}

fn parse_message_record(
    ordinal: u64,
    object: &serde_json::Map<String, Value>,
) -> Result<CursorTranscriptRecord> {
    require_only_keys(ordinal, object, &["role", "message"])?;
    let role = match object.get("role") {
        Some(Value::String(role)) => match role.as_str() {
            "user" => CursorMessageRole::User,
            "assistant" => CursorMessageRole::Assistant,
            other => bail!("cursor transcript record {ordinal} has unobserved role '{other}'"),
        },
        _ => bail!("cursor transcript record {ordinal} field 'role' is not a string"),
    };
    let Some(Value::Object(message)) = object.get("message") else {
        bail!("cursor transcript record {ordinal} field 'message' is not an object");
    };
    require_only_keys(ordinal, message, &["content"])?;
    let Some(Value::Array(blocks)) = message.get("content") else {
        bail!("cursor transcript record {ordinal} field 'message.content' is not an array");
    };
    let mut text_parts: Vec<&str> = Vec::new();
    let mut tool_use_names = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        let Value::Object(block) = block else {
            bail!(
                "cursor transcript record {ordinal} content block {block_index} is not an object"
            );
        };
        match block.get("type") {
            Some(Value::String(kind)) if kind == "text" => {
                require_only_keys(ordinal, block, &["type", "text"])?;
                let Some(Value::String(block_text)) = block.get("text") else {
                    bail!(
                        "cursor transcript record {ordinal} text block {block_index} has no string text"
                    );
                };
                text_parts.push(block_text);
            }
            Some(Value::String(kind)) if kind == "tool_use" => {
                if role != CursorMessageRole::Assistant {
                    bail!(
                        "cursor transcript record {ordinal} has a tool_use block on a non-assistant role (unobserved grammar)"
                    );
                }
                require_only_keys(ordinal, block, &["type", "name", "input"])?;
                let Some(Value::String(name)) = block.get("name") else {
                    bail!(
                        "cursor transcript record {ordinal} tool_use block {block_index} has no string name"
                    );
                };
                if name.trim().is_empty() {
                    bail!(
                        "cursor transcript record {ordinal} tool_use block {block_index} has an empty name"
                    );
                }
                if !matches!(block.get("input"), Some(Value::Object(_))) {
                    bail!(
                        "cursor transcript record {ordinal} tool_use block {block_index} has a non-object input"
                    );
                }
                tool_use_names.push(name.clone());
            }
            Some(Value::String(kind)) => bail!(
                "cursor transcript record {ordinal} content block {block_index} has unobserved type '{kind}'"
            ),
            _ => bail!(
                "cursor transcript record {ordinal} content block {block_index} has no string type"
            ),
        }
    }
    Ok(CursorTranscriptRecord::Message {
        ordinal,
        role,
        text: text_parts.join("\n"),
        tool_use_names,
    })
}

fn parse_turn_ended_record(
    ordinal: u64,
    object: &serde_json::Map<String, Value>,
) -> Result<CursorTranscriptRecord> {
    match object.get("type") {
        Some(Value::String(kind)) if kind == "turn_ended" => {}
        Some(Value::String(kind)) => {
            bail!("cursor transcript record {ordinal} has unobserved type '{kind}'")
        }
        _ => bail!("cursor transcript record {ordinal} field 'type' is not a string"),
    }
    let status = match object.get("status") {
        Some(Value::String(status)) => match status.as_str() {
            "success" => CursorTurnEndedStatus::Success,
            "error" => CursorTurnEndedStatus::Error,
            other => bail!(
                "cursor transcript record {ordinal} has unobserved turn_ended status '{other}'"
            ),
        },
        _ => bail!("cursor transcript record {ordinal} turn_ended has no string status"),
    };
    let error = match (status, object.get("error")) {
        (CursorTurnEndedStatus::Error, Some(Value::String(error))) => Some(error.clone()),
        (CursorTurnEndedStatus::Error, None) => None,
        (CursorTurnEndedStatus::Success, None) => None,
        _ => bail!("cursor transcript record {ordinal} turn_ended has an invalid error field"),
    };
    let allowed: &[&str] = match status {
        CursorTurnEndedStatus::Success => &["type", "status"],
        CursorTurnEndedStatus::Error => &["type", "status", "error"],
    };
    require_only_keys(ordinal, object, allowed)?;
    Ok(CursorTranscriptRecord::TurnEnded {
        ordinal,
        status,
        error,
    })
}

fn require_only_keys(
    ordinal: u64,
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<()> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!(
                "cursor transcript record {ordinal} carries unobserved key '{key}'; \
                 the versioned grammar fails closed instead of guessing"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PR #874 (Cursor 3.6.31): two `{role,message}` records with only
    /// top-level `role` and `message`, text in `message.content[].text`.
    const PR874_STYLE: &str = concat!(
        "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello cursor\"}]}}\n",
        "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi there\"}]}}\n",
    );

    fn pr914_fixture() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/research/fixtures/cursor-hooks-contract-2026-07-23/transcript.synthetic.jsonl"
        );
        std::fs::read(path).expect("read PR #914 synthetic transcript fixture")
    }

    #[test]
    fn pr874_two_record_positive_case_needs_no_per_message_identity() {
        let transcript = parse_cursor_transcript(PR874_STYLE.as_bytes())
            .expect("PR #874 grammar is a valid positive case");
        assert_eq!(transcript.records.len(), 2);
        assert_eq!(transcript.snapshot_byte_len, PR874_STYLE.len() as u64);
        let messages: Vec<_> = transcript.usable_messages().collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, 0);
        assert_eq!(messages[0].1, CursorMessageRole::User);
        assert_eq!(messages[0].2, "hello cursor");
        assert_eq!(messages[1].0, 1);
        assert_eq!(transcript.last_assistant_text(), Some("hi there"));
    }

    #[test]
    fn pr914_fixture_grammar_orders_all_records_by_physical_ordinal() {
        let bytes = pr914_fixture();
        let transcript =
            parse_cursor_transcript(&bytes).expect("PR #914 grammar is a valid positive case");
        assert_eq!(transcript.records.len(), 7);
        for (index, record) in transcript.records.iter().enumerate() {
            let ordinal = match record {
                CursorTranscriptRecord::Message { ordinal, .. } => *ordinal,
                CursorTranscriptRecord::TurnEnded { ordinal, .. } => *ordinal,
            };
            assert_eq!(
                ordinal, index as u64,
                "physical ordinal covers every record"
            );
        }
        // Assistant tool_use block is preserved in the IR without becoming text.
        let CursorTranscriptRecord::Message {
            role,
            text,
            tool_use_names,
            ..
        } = &transcript.records[1]
        else {
            panic!("record 1 must be an assistant message");
        };
        assert_eq!(*role, CursorMessageRole::Assistant);
        assert!(text.contains("<synthetic-tool-preamble>"));
        assert_eq!(tool_use_names, &["Read".to_string()]);
        // Standalone turn_ended records occupy ordinals 3 and 6, so the
        // message ordinal sequence has holes but is never compacted.
        assert!(matches!(
            transcript.records[3],
            CursorTranscriptRecord::TurnEnded {
                ordinal: 3,
                status: CursorTurnEndedStatus::Success,
                error: None,
            }
        ));
        let CursorTranscriptRecord::TurnEnded {
            ordinal: 6,
            status: CursorTurnEndedStatus::Error,
            error: Some(error),
        } = &transcript.records[6]
        else {
            panic!("record 6 must be the aborted-turn boundary");
        };
        assert_eq!(error, "User aborted request");
        let ordinals: Vec<u64> = transcript
            .usable_messages()
            .map(|(ordinal, _, _)| ordinal)
            .collect();
        assert_eq!(ordinals, vec![0, 1, 2, 4, 5]);
    }

    #[test]
    fn turn_ended_records_never_enter_the_usable_message_projection() {
        let transcript = parse_cursor_transcript(&pr914_fixture()).expect("valid fixture");
        assert!(transcript.usable_messages().all(|(_, role, _)| matches!(
            role,
            CursorMessageRole::User | CursorMessageRole::Assistant
        )));
        assert_eq!(transcript.usable_messages().count(), 5);
    }

    #[test]
    fn duplicate_same_role_same_text_records_keep_distinct_ordinals() {
        let input = concat!(
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"again\"}]}}\n",
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"again\"}]}}\n",
        );
        let transcript = parse_cursor_transcript(input.as_bytes()).expect("duplicates are valid");
        let messages: Vec<_> = transcript.usable_messages().collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, 0);
        assert_eq!(messages[1].0, 1);
        assert_eq!(messages[0].2, messages[1].2);
    }

    #[test]
    fn any_broken_record_invalidates_the_whole_input() {
        let broken_middle = concat!(
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n",
            "{not-json\n",
            "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n",
        );
        assert!(parse_cursor_transcript(broken_middle.as_bytes()).is_err());

        let broken_tail = concat!(
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n",
            "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\"",
        );
        assert!(parse_cursor_transcript(broken_tail.as_bytes()).is_err());
    }

    #[test]
    fn unobserved_shapes_fail_closed_without_synthesis() {
        for input in [
            // unknown top-level key
            "{\"role\":\"user\",\"message\":{\"content\":[]},\"id\":\"m1\"}",
            // unknown record type
            "{\"type\":\"session_meta\",\"status\":\"success\"}",
            // unknown block type
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"image\",\"text\":\"x\"}]}}",
            // tool_use on a user message
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{}}]}}",
            // unknown role
            "{\"role\":\"system\",\"message\":{\"content\":[]}}",
            // unknown turn_ended status
            "{\"type\":\"turn_ended\",\"status\":\"cancelled\"}",
            // Claude/Codex-style record must never validate here
            "{\"type\":\"user\",\"sessionId\":\"s\",\"message\":{\"content\":\"hi\"}}",
        ] {
            assert!(
                parse_cursor_transcript(input.as_bytes()).is_err(),
                "input must fail closed: {input}"
            );
        }
    }

    #[test]
    fn empty_and_non_utf8_inputs_fail_or_stay_unusable() {
        let empty = parse_cursor_transcript(b"").expect("zero records parse");
        assert_eq!(empty.records.len(), 0);
        assert_eq!(empty.usable_messages().count(), 0);
        assert!(parse_cursor_transcript(&[0xff, 0xfe, 0x00]).is_err());
        assert!(parse_cursor_transcript(b"\n").is_err());
    }
}
