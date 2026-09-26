use std::io::{BufReader, Read};

use serde_json::Value;

use super::raw_archive::{ROLE_ASSISTANT, ROLE_USER};

pub(crate) const CODEX_TRANSCRIPT_MESSAGE_TOOL: &str = "codex-transcript";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedTranscriptMessage {
    pub role: &'static str,
    pub text: String,
    pub created_at_epoch: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TranscriptRecordClass {
    Conversation(ParsedTranscriptMessage),
    MetaUser(ParsedTranscriptMessage),
    XmlControlUser(ParsedTranscriptMessage),
    MissingEventTime(ParsedTranscriptMessage),
    EmptyText,
    UnsupportedRecord,
    MalformedRecord,
    OutsideWindow,
}

pub(crate) fn classify_transcript_line(
    line: &str,
    window: Option<(i64, i64)>,
) -> TranscriptRecordClass {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return TranscriptRecordClass::MalformedRecord;
    };
    let event_epoch = transcript_timestamp_epoch(&value);
    if event_epoch
        .is_some_and(|epoch| window.is_some_and(|(since, until)| epoch < since || epoch > until))
    {
        return TranscriptRecordClass::OutsideWindow;
    }
    let Some(message) = parse_transcript_message(&value) else {
        return TranscriptRecordClass::UnsupportedRecord;
    };
    if event_epoch.is_none() {
        return TranscriptRecordClass::MissingEventTime(message);
    }
    if message.text.trim().is_empty() {
        return TranscriptRecordClass::EmptyText;
    }
    if message.role == ROLE_USER && transcript_is_meta(&value) {
        return TranscriptRecordClass::MetaUser(message);
    }
    if message.role == ROLE_USER && message.text.trim_start().starts_with('<') {
        return TranscriptRecordClass::XmlControlUser(message);
    }
    TranscriptRecordClass::Conversation(message)
}

fn transcript_is_meta(value: &Value) -> bool {
    value
        .get("isMeta")
        .or_else(|| value.get("is_meta"))
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("isMeta"))
        })
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("is_meta"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
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

pub(crate) fn stream_captured_transcript(
    file: std::fs::File,
    byte_limit: u64,
    mut visit: impl FnMut(&str, bool),
) -> std::io::Result<()> {
    stream_reader(file.take(byte_limit), Some(byte_limit), &mut visit)
}

fn stream_reader(
    reader: impl Read,
    expected_bytes: Option<u64>,
    visit: &mut impl FnMut(&str, bool),
) -> std::io::Result<()> {
    let options = agent_sessions::RawReadOptions {
        // The caller already caps the input. Validate length after framing so
        // an unterminated short tail still releases the preceding pending row.
        stop_at_byte: None,
        max_read_bytes: None,
        max_line_bytes: None,
        ..Default::default()
    };
    let reader = agent_sessions::read_raw_from(BufReader::new(reader), &options)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let mut pending = None;
    let mut total_bytes = 0;
    for record in reader {
        let record = record.map_err(|error| match error {
            agent_sessions::StreamError::Io(error) => error,
            error => std::io::Error::new(std::io::ErrorKind::InvalidData, error),
        })?;
        total_bytes = record.byte_end;
        if let Some(line) = pending.replace(record.bytes) {
            visit_line(&line, false, visit)?;
        }
    }
    if let Some(expected) = expected_bytes {
        if total_bytes != expected {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, format!(
                "transcript truncated before captured boundary: expected {expected} bytes, read {total_bytes}"
            )));
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
    let message = agent_sessions::project_conversation(value)?;
    let role = match message.role {
        agent_sessions::Role::User => ROLE_USER,
        agent_sessions::Role::Assistant => ROLE_ASSISTANT,
        _ => return None,
    };
    Some(ParsedTranscriptMessage {
        role,
        text: message.text,
        created_at_epoch: message.created_at_epoch,
    })
}

pub(crate) fn transcript_timestamp_epoch(value: &Value) -> Option<i64> {
    agent_sessions::tolerant_timestamp_epoch(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Catches eager delivery of the pending final row on short captured reads.
    #[test]
    fn short_capture_withholds_pending_record() {
        let mut rows = Vec::new();
        let error = stream_reader(&b"one\r\ntwo"[..], Some(20), &mut |line, final_row| {
            rows.push((line.to_owned(), final_row));
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert_eq!(
            error.to_string(),
            "transcript truncated before captured boundary: expected 20 bytes, read 8"
        );
        assert_eq!(rows, vec![("one".to_owned(), false)]);
    }

    // Catches dropping empty physical rows or accepting invalid UTF-8 lossily.
    #[test]
    fn raw_adapter_preserves_empty_rows_and_rejects_invalid_utf8() {
        let mut rows = Vec::new();
        stream_reader(&b"\r\nlast\r"[..], None, &mut |line, final_row| {
            rows.push((line.to_owned(), final_row));
        })
        .unwrap();
        assert_eq!(
            rows,
            vec![(String::new(), false), ("last".to_owned(), true)]
        );
        let mut rows = Vec::new();
        let error = stream_reader(&b"ok\n\xff\n"[..], None, &mut |line, final_row| {
            rows.push((line.to_owned(), final_row));
        })
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(rows, vec![("ok".to_owned(), false)]);
    }

    // Catches strict timestamp fallback, ignored metadata or altered block joining.
    #[test]
    fn tolerant_projection_retains_native_message_policy() {
        let value = serde_json::json!({"type":"user", "timestamp":null, "created_at":123,
            "isMeta":true, "message":{"content":[{"type":"input_text","text":"a"},
                {"type":"image","text":"ignored"},{"type":"output_text","text":"b"},
                {"type":"text","text":4}]}});
        assert_eq!(
            parse_transcript_message(&value),
            Some(ParsedTranscriptMessage {
                role: ROLE_USER,
                text: "a\nb".into(),
                created_at_epoch: None,
            })
        );
        assert_eq!(
            parse_transcript_message(&serde_json::json!({"type":"assistant", "createdAt":" -5 "})),
            Some(ParsedTranscriptMessage {
                role: ROLE_ASSISTANT,
                text: String::new(),
                created_at_epoch: Some(-5)
            })
        );
    }

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

    #[test]
    fn classifier_applies_window_and_disjoint_exclusion_precedence() {
        assert_eq!(
            classify_transcript_line(
                r#"{"timestamp":99,"type":"user","isMeta":true,"message":{"content":"secret"}}"#,
                Some((100, 200))
            ),
            TranscriptRecordClass::OutsideWindow
        );
        assert!(matches!(
            classify_transcript_line(
                r#"{"type":"user","isMeta":true,"message":{"content":""}}"#,
                Some((100, 200))
            ),
            TranscriptRecordClass::MissingEventTime(_)
        ));
        assert!(matches!(
            classify_transcript_line(
                r#"{"timestamp":100,"type":"user","isMeta":true,"message":{"content":"meta"}}"#,
                Some((100, 200))
            ),
            TranscriptRecordClass::MetaUser(_)
        ));
        assert!(matches!(
            classify_transcript_line(
                r#"{"timestamp":100,"type":"user","message":{"content":"  <system>"}}"#,
                Some((100, 200))
            ),
            TranscriptRecordClass::XmlControlUser(_)
        ));
    }

    #[test]
    fn captured_boundary_excludes_post_capture_append() -> std::io::Result<()> {
        use std::io::Write;

        let path = std::env::temp_dir().join(format!(
            "remem-captured-boundary-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, b"{\"type\":\"progress\",\"timestamp\":100}\n")?;
        let file = std::fs::File::open(&path)?;
        let byte_limit = file.metadata()?.len();
        let mut append = std::fs::OpenOptions::new().append(true).open(&path)?;
        append.write_all(b"{\"type\":\"progress\",\"timestamp\":101}\n")?;
        append.flush()?;
        let mut lines = Vec::new();

        stream_captured_transcript(file, byte_limit, |line, _| {
            lines.push(line.to_string());
        })?;

        assert_eq!(lines, vec![r#"{"type":"progress","timestamp":100}"#]);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
