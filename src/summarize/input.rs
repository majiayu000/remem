use anyhow::Result;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Deserialize)]
pub(super) struct SummarizeInput {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    pub last_assistant_message: Option<String>,
}

pub(super) fn hash_message(msg: &str) -> String {
    let mut hasher = DefaultHasher::new();
    msg.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) fn extract_last_assistant_message(transcript_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(transcript_path).ok()?;

    for line in content.lines().rev() {
        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if val["type"].as_str() != Some("assistant") {
            continue;
        }
        let Some(content_arr) = val["message"]["content"].as_array() else {
            continue;
        };
        let text_parts: Vec<&str> = content_arr
            .iter()
            .filter_map(|entry| {
                if entry["type"].as_str() == Some("text") {
                    entry["text"].as_str()
                } else {
                    None
                }
            })
            .collect();
        if !text_parts.is_empty() {
            return Some(text_parts.join("\n"));
        }
    }

    None
}

pub(super) fn read_stdin_with_timeout(timeout_ms: u64) -> Result<Option<String>> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let input = std::io::read_to_string(std::io::stdin());
        let _ = tx.send(input);
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(Ok(input)) => {
            if input.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(input))
            }
        }
        Ok(Err(err)) => Err(err.into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            crate::log::warn(
                "summarize",
                &format!("stdin read timed out after {}ms, skipping", timeout_ms),
            );
            Ok(None)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            crate::log::warn("summarize", "stdin reader disconnected, skipping");
            Ok(None)
        }
    }
}
