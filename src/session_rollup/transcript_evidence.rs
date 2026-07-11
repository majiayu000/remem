use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::db;

use super::side_effects::{
    stop_payloads, stop_transcript_path, unique_transcript_payload_indices, StopHookPayload,
};
use super::RollupRange;

pub(super) const TRANSCRIPT_MESSAGE_CONTENT_LIMIT: usize = 8 * 1024;
pub(super) const TRANSCRIPT_TOTAL_CONTENT_LIMIT: usize = 64 * 1024;
pub(super) const TRANSCRIPT_MESSAGE_COUNT_LIMIT: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PromptTranscriptMessage {
    pub(super) source_event_id: i64,
    pub(super) role: String,
    pub(super) content: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PromptTranscriptEvidence {
    pub(super) messages: Vec<PromptTranscriptMessage>,
    pub(super) truncated: bool,
}

impl PromptTranscriptEvidence {
    pub(super) fn validate_for_range(&self, range: &RollupRange) -> Result<()> {
        if self.messages.len() > TRANSCRIPT_MESSAGE_COUNT_LIMIT {
            bail!("invalid payload: persisted transcript evidence exceeds message-count budget");
        }
        let total_bytes = self
            .messages
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>();
        if total_bytes > TRANSCRIPT_TOTAL_CONTENT_LIMIT {
            bail!("invalid payload: persisted transcript evidence exceeds total byte budget");
        }
        let stop_event_ids = range
            .events
            .iter()
            .filter(|event| event.event_type == "session_stop")
            .map(|event| event.id)
            .collect::<BTreeSet<_>>();
        for message in &self.messages {
            if !stop_event_ids.contains(&message.source_event_id) {
                bail!(
                    "invalid payload: persisted transcript evidence event {} is not a Stop event in the exact rollup range",
                    message.source_event_id
                );
            }
            if !matches!(message.role.as_str(), "user" | "assistant") {
                bail!(
                    "invalid payload: persisted transcript evidence has unsupported role '{}'",
                    message.role
                );
            }
            if message.content.trim().is_empty() {
                bail!("invalid payload: persisted transcript evidence contains an empty message");
            }
            if message.content.len() > TRANSCRIPT_MESSAGE_CONTENT_LIMIT {
                bail!(
                    "invalid payload: persisted transcript evidence exceeds per-message byte budget"
                );
            }
            if crate::adapter::common::redact_sensitive_text(&message.content) != message.content {
                bail!("invalid payload: persisted transcript evidence is not redacted");
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct EvidenceBudget {
    evidence: PromptTranscriptEvidence,
    total_bytes: usize,
}

impl EvidenceBudget {
    fn push(&mut self, mut message: PromptTranscriptMessage) {
        let redacted = crate::adapter::common::redact_sensitive_text(&message.content);
        let bounded = db::truncate_str(&redacted, TRANSCRIPT_MESSAGE_CONTENT_LIMIT);
        if bounded.len() < redacted.len() {
            self.evidence.truncated = true;
        }
        message.content = bounded.to_string();
        if message.content.is_empty() {
            return;
        }
        self.total_bytes += message.content.len();
        self.evidence.messages.push(message);

        while self.evidence.messages.len() > TRANSCRIPT_MESSAGE_COUNT_LIMIT {
            self.total_bytes -= self.evidence.messages.remove(0).content.len();
            self.evidence.truncated = true;
        }
        while self.total_bytes > TRANSCRIPT_TOTAL_CONTENT_LIMIT {
            let excess = self.total_bytes - TRANSCRIPT_TOTAL_CONTENT_LIMIT;
            let first_len = self.evidence.messages[0].content.len();
            if first_len <= excess {
                self.total_bytes -= self.evidence.messages.remove(0).content.len();
            } else {
                let keep_bytes = first_len - excess;
                let shortened =
                    db::truncate_str(&self.evidence.messages[0].content, keep_bytes).to_string();
                self.total_bytes -= first_len - shortened.len();
                self.evidence.messages[0].content = shortened;
            }
            self.evidence.truncated = true;
        }
    }

    fn finish(self) -> PromptTranscriptEvidence {
        self.evidence
    }
}

#[cfg(test)]
pub(super) fn bound_prompt_transcript_evidence(
    messages: impl IntoIterator<Item = PromptTranscriptMessage>,
) -> PromptTranscriptEvidence {
    let mut budget = EvidenceBudget::default();
    for message in messages {
        budget.push(message);
    }
    budget.finish()
}

pub(super) fn load_prompt_transcript_evidence(
    range: &RollupRange,
) -> Result<PromptTranscriptEvidence> {
    let payloads = stop_payloads(range)?;
    let selected_transcripts = unique_transcript_payload_indices(&payloads);
    let represented_text = captured_event_text(range);
    let captured_conversation_available = has_captured_conversation(range, &payloads);
    let mut budget = EvidenceBudget::default();

    for payload_index in selected_transcripts {
        let payload = &payloads[payload_index];
        let Some(transcript_path) = stop_transcript_path(payload) else {
            continue;
        };
        let Some(transcript_byte_len) = payload.transcript_byte_len else {
            if captured_conversation_available {
                crate::log::info(
                    "session-rollup",
                    &format!(
                        "legacy Stop event {} has no transcript_byte_len; using captured conversational events only for rollup evidence",
                        payload.source_event_id
                    ),
                );
                continue;
            }
            bail!(
                "missing evidence: captured event {} has transcript_path but no transcript_byte_len or captured user/assistant fallback",
                payload.source_event_id
            );
        };
        let content = crate::memory::raw_transcript::read_transcript_content(
            transcript_path,
            Some(transcript_byte_len),
        )
        .with_context(|| {
            format!(
                "read bounded transcript prompt evidence for captured event {}",
                payload.source_event_id
            )
        })?;
        let mut has_usable_message = false;

        for (line_index, line) in content.lines().enumerate() {
            let value = serde_json::from_str::<serde_json::Value>(line).with_context(|| {
                format!(
                    "parse bounded transcript prompt evidence for captured event {} line {}",
                    payload.source_event_id,
                    line_index + 1
                )
            })?;
            let Some(message) = crate::memory::raw_transcript::parse_transcript_message(&value)
            else {
                continue;
            };
            let text = message.text.trim();
            if text.is_empty() {
                continue;
            }
            has_usable_message = true;
            let redacted = crate::adapter::common::redact_sensitive_text(text);
            if represented_text.contains(text) || represented_text.contains(redacted.trim()) {
                continue;
            }
            budget.push(PromptTranscriptMessage {
                source_event_id: payload.source_event_id,
                role: message.role.to_string(),
                content: text.to_string(),
            });
        }
        if !has_usable_message {
            bail!(
                "bounded transcript prompt evidence for captured event {} contains no usable user or assistant messages",
                payload.source_event_id
            );
        }
    }
    Ok(budget.finish())
}

fn has_captured_conversation(range: &RollupRange, payloads: &[StopHookPayload]) -> bool {
    range.events.iter().any(|event| {
        !event.content.trim().is_empty()
            && (matches!(event.role.as_deref(), Some("user" | "assistant"))
                || event.event_type == "user_prompt_submit")
    }) || payloads.iter().any(|payload| {
        payload
            .last_assistant_message
            .as_deref()
            .is_some_and(|message| !message.trim().is_empty())
    })
}

fn captured_event_text(range: &RollupRange) -> BTreeSet<String> {
    let mut text = BTreeSet::new();
    for event in &range.events {
        let content = event.content.trim();
        if !content.is_empty() {
            text.insert(content.to_string());
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&event.content) {
            collect_json_text(&value, &mut text);
        }
    }
    text
}

fn collect_json_text(value: &serde_json::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(fields) => {
            for value in fields.values() {
                collect_json_text(value, out);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_text(value, out);
            }
        }
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                out.insert(trimmed.to_string());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_rollup::{RollupEvent, RollupRange};

    fn range_with_stop() -> RollupRange {
        RollupRange {
            from_event_id: 1,
            to_event_id: 2,
            events: vec![
                RollupEvent {
                    id: 1,
                    event_type: "user_prompt_submit".to_string(),
                    role: Some("user".to_string()),
                    tool_name: None,
                    content: "captured request".to_string(),
                    token_estimate: 4,
                    created_at_epoch: 1,
                    turn_id: None,
                },
                RollupEvent {
                    id: 2,
                    event_type: "session_stop".to_string(),
                    role: None,
                    tool_name: None,
                    content: "{}".to_string(),
                    token_estimate: 1,
                    created_at_epoch: 2,
                    turn_id: None,
                },
            ],
        }
    }

    #[test]
    fn persisted_evidence_validation_requires_stop_anchor_and_redacted_text() {
        let range = range_with_stop();
        let mut evidence = PromptTranscriptEvidence {
            messages: vec![PromptTranscriptMessage {
                source_event_id: 1,
                role: "user".to_string(),
                content: "bounded conversation text".to_string(),
            }],
            truncated: false,
        };

        let anchor_error = evidence
            .validate_for_range(&range)
            .expect_err("non-Stop anchors must fail closed");
        assert!(anchor_error.to_string().contains("not a Stop event"));

        evidence.messages[0].source_event_id = 2;
        evidence.messages[0].content = "password=hunter2".to_string();
        let redaction_error = evidence
            .validate_for_range(&range)
            .expect_err("unredacted persisted text must fail closed");
        assert!(redaction_error.to_string().contains("not redacted"));
    }

    #[test]
    fn persisted_evidence_json_rejects_unknown_fields() {
        let error = serde_json::from_str::<PromptTranscriptEvidence>(
            r#"{"messages":[],"truncated":false,"unexpected":true}"#,
        )
        .expect_err("unknown persisted evidence fields must fail closed");

        assert!(error.to_string().contains("unknown field"));
    }
}
