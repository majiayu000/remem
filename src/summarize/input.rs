use anyhow::Result;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Cursor `remem summarize` entrypoint (GH-823 B-008, SP823-T5 gate).
///
/// Cursor Stop summarization is a hard prerequisite of GH-825's verified
/// Cursor transcript reader. Until that lands, this path is fail-closed: the
/// bounded reader and exact `stop` event validation run first, then the
/// command returns a non-zero explicit unsupported error with zero enqueue,
/// spill, database, transcript-reader, or LLM calls. A Cursor transcript
/// path must never reach the Claude/Codex raw transcript parser.
pub async fn summarize_cursor() -> Result<()> {
    let bytes = crate::cursor_hook::input::read_bounded_hook_stdin(&mut std::io::stdin().lock())?;
    summarize_cursor_bytes(&bytes)
}

pub fn summarize_cursor_bytes(bytes: &[u8]) -> Result<()> {
    crate::cursor_hook::input::require_stop_event(bytes)?;
    crate::log::error(
        "summarize",
        "cursor summarize is blocked on GH-825's Cursor transcript reader; failing closed with zero side effects",
    );
    anyhow::bail!(
        "cursor summarize is unsupported until GH-825's Cursor transcript reader is merged; \
         failing closed with zero side effects"
    )
}

#[derive(Debug, Deserialize)]
pub(super) struct SummarizeInput {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    pub transcript_byte_len: Option<u64>,
    pub last_assistant_message: Option<String>,
}

pub(crate) fn hash_message(msg: &str) -> String {
    let mut hasher = DefaultHasher::new();
    msg.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) fn extract_last_assistant_message(transcript_path: &str) -> Option<String> {
    extract_last_assistant_message_with_limit(transcript_path, None)
}

pub(crate) fn extract_last_assistant_message_with_limit(
    transcript_path: &str,
    byte_limit: Option<u64>,
) -> Option<String> {
    let content =
        crate::memory::raw_transcript::read_transcript_content(transcript_path, byte_limit).ok()?;

    for line in content.lines().rev() {
        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(message) = crate::memory::raw_transcript::parse_transcript_message(&val) else {
            continue;
        };
        if message.role != crate::memory::raw_archive::ROLE_ASSISTANT {
            continue;
        }
        if !message.text.trim().is_empty() {
            return Some(message.text);
        }
    }

    None
}
