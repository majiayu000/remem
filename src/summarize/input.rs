use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

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
