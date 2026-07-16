use std::io::{BufRead, BufReader, Read};

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

/// Visit a transcript one line at a time while retaining at most the current
/// and next JSONL records. A captured byte boundary is treated as an immutable
/// snapshot: shorter files fail instead of silently draining a later shape.
pub(crate) fn stream_transcript_lines(
    transcript_path: &str,
    byte_limit: Option<u64>,
    mut visit: impl FnMut(&str, bool),
) -> std::io::Result<()> {
    let file = std::fs::File::open(transcript_path)?;
    match byte_limit {
        Some(limit) => stream_reader(file.take(limit), Some(limit), &mut visit),
        None => stream_reader(file, None, &mut visit),
    }
}

fn stream_reader(
    reader: impl Read,
    expected_bytes: Option<u64>,
    visit: &mut impl FnMut(&str, bool),
) -> std::io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut pending = None;
    let mut total_bytes = 0_u64;

    loop {
        let mut next = Vec::new();
        let bytes_read = reader.read_until(b'\n', &mut next)?;
        if bytes_read == 0 {
            break;
        }
        total_bytes = total_bytes.checked_add(bytes_read as u64).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "transcript size overflow")
        })?;
        if let Some(line) = pending.replace(next) {
            visit_line(&line, false, visit)?;
        }
    }

    if let Some(expected) = expected_bytes {
        if total_bytes != expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "transcript truncated before captured boundary: expected {expected} bytes, read {total_bytes}"
                ),
            ));
        }
    }
    if let Some(line) = pending {
        visit_line(&line, true, visit)?;
    }
    Ok(())
}

fn visit_line(
    bytes: &[u8],
    is_final: bool,
    visit: &mut impl FnMut(&str, bool),
) -> std::io::Result<()> {
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    let line = std::str::from_utf8(&bytes[..end])
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    visit(line, is_final);
    Ok(())
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
