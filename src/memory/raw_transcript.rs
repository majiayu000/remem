use serde_json::Value;

use super::raw_archive::{ROLE_ASSISTANT, ROLE_USER};

pub(crate) struct ParsedTranscriptMessage {
    pub role: &'static str,
    pub text: String,
}

pub(crate) fn parse_transcript_message(value: &Value) -> Option<ParsedTranscriptMessage> {
    match value.get("type").and_then(Value::as_str)? {
        "user" => Some(ParsedTranscriptMessage {
            role: ROLE_USER,
            text: extract_content_text(&value["message"]["content"]),
        }),
        "assistant" => Some(ParsedTranscriptMessage {
            role: ROLE_ASSISTANT,
            text: extract_content_text(&value["message"]["content"]),
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
}
