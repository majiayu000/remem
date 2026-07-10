use std::io::Read;

use serde_json::Value;

use super::raw_archive::{ROLE_ASSISTANT, ROLE_USER};

pub(crate) struct ParsedTranscriptMessage {
    pub role: &'static str,
    pub text: String,
    pub created_at_epoch: Option<i64>,
}

pub(crate) fn read_transcript_content(
    transcript_path: &str,
    byte_limit: Option<u64>,
) -> std::io::Result<String> {
    let Some(byte_limit) = byte_limit else {
        return std::fs::read_to_string(transcript_path);
    };
    let file = std::fs::File::open(transcript_path)?;
    let mut content = String::new();
    file.take(byte_limit).read_to_string(&mut content)?;
    if content.len() as u64 != byte_limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            format!(
                "transcript truncated before captured boundary: expected {byte_limit} bytes, read {}",
                content.len()
            ),
        ));
    }
    Ok(content)
}

pub(crate) fn parse_transcript_message(value: &Value) -> Option<ParsedTranscriptMessage> {
    let created_at_epoch = transcript_timestamp_epoch(value);
    match value.get("type").and_then(Value::as_str)? {
        "user" => Some(ParsedTranscriptMessage {
            role: ROLE_USER,
            text: extract_content_text(&value["message"]["content"]),
            created_at_epoch,
        }),
        "assistant" => Some(ParsedTranscriptMessage {
            role: ROLE_ASSISTANT,
            text: extract_content_text(&value["message"]["content"]),
            created_at_epoch,
        }),
        "response_item" => parse_codex_response_item(value),
        _ => None,
    }
}

fn parse_codex_response_item(value: &Value) -> Option<ParsedTranscriptMessage> {
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    Some(ParsedTranscriptMessage {
        role: transcript_role(payload.get("role").and_then(Value::as_str)?)?,
        text: extract_content_text(&payload["content"]),
        created_at_epoch: transcript_timestamp_epoch(value),
    })
}

fn transcript_role(role: &str) -> Option<&'static str> {
    match role {
        "user" => Some(ROLE_USER),
        "assistant" => Some(ROLE_ASSISTANT),
        _ => None,
    }
}

fn extract_content_text(content: &Value) -> String {
    if let Some(array) = content.as_array() {
        let parts: Vec<String> = array
            .iter()
            .filter_map(|entry| match entry.get("type").and_then(Value::as_str) {
                Some("text" | "input_text" | "output_text") => entry
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            })
            .collect();
        return parts.join("\n");
    }
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    String::new()
}

fn transcript_timestamp_epoch(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("timestamp"))
        })
        .and_then(parse_timestamp_value)
}

fn parse_timestamp_value(value: &Value) -> Option<i64> {
    if let Some(epoch) = value.as_i64() {
        return Some(epoch);
    }
    let text = value.as_str()?.trim();
    if let Ok(epoch) = text.parse::<i64>() {
        return Some(epoch);
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|datetime| datetime.timestamp())
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_claude_message_shape() {
        let value: Value = serde_json::from_str(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"kept"}]}}"#,
        )
        .unwrap();

        let parsed = parse_transcript_message(&value).expect("message should parse");

        assert_eq!(parsed.role, ROLE_ASSISTANT);
        assert_eq!(parsed.text, "kept");
        assert_eq!(parsed.created_at_epoch, None);
    }

    #[test]
    fn parses_codex_rollout_response_item_shape() {
        let mut roles = Vec::new();
        let mut texts = Vec::new();
        for line in include_str!("../../tests/fixtures/codex-rollout-minimal.jsonl").lines() {
            let value: Value = serde_json::from_str(line).unwrap();
            if let Some(parsed) = parse_transcript_message(&value) {
                roles.push(parsed.role);
                texts.push(parsed.text);
            }
        }

        assert_eq!(roles, vec![ROLE_USER, ROLE_ASSISTANT]);
        assert_eq!(
            texts,
            vec![
                "Codex rollout user text should enter the raw archive.",
                "Codex rollout assistant text should enter the raw archive."
            ]
        );
    }

    #[test]
    fn parses_transcript_timestamp_epoch() {
        let value: Value = serde_json::from_str(
            r#"{"timestamp":"2026-06-12T00:00:03.000Z","type":"assistant","message":{"content":"kept"}}"#,
        )
        .unwrap();

        let parsed = parse_transcript_message(&value).expect("message should parse");

        assert_eq!(parsed.created_at_epoch, Some(1_781_222_403));
    }
}
