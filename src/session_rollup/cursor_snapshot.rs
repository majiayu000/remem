//! Stop-time Cursor transcript snapshot capture and explicit degradation
//! (GH-825).
//!
//! The snapshot is taken inside the short-lived Stop hook invocation, keyed
//! by the canonical Stop key; the SessionRollup worker never reopens the
//! original transcript path. Every failure maps to a stable, machine-readable
//! `degraded/<reason>` marker (GH-825 transcript failure matrix) instead of
//! dropping the Stop or silently pretending success. The marker travels
//! inside the durable `session_stop` capture payload, so fidelity is always
//! auditable from the ledger.

use serde::{Deserialize, Serialize};

use super::cursor_transcript::{parse_cursor_transcript, ValidatedCursorTranscript};

/// Maximum accepted Cursor transcript snapshot size in bytes.
///
/// #822 has not frozen a real-host maximum yet (observed foreground
/// transcripts were under 2 KiB); this value is a conservative
/// implementation proposal pending explicit human confirmation on issue
/// #825, mirroring how `CURSOR_TOOL_FIELD_MAX_BYTES` is staged.
pub(crate) const CURSOR_TRANSCRIPT_MAX_BYTES: u64 = 2 * 1024 * 1024;

pub(crate) const CURSOR_FIDELITY_FULL: &str = "full";
pub(crate) const CURSOR_FIDELITY_DEGRADED: &str = "degraded";

/// Machine-readable Cursor capture fidelity marker embedded in the
/// `session_stop` payload (`cursor_capture` field). No new schema: the
/// marker rides the existing captured-event content/blob path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CursorCaptureMarker {
    /// `full` or `degraded`.
    pub(crate) fidelity: String,
    /// Stable degradation reason code (`None` when `full`).
    pub(crate) reason_code: Option<String>,
    /// Approved Stop status (`completed` or `aborted`).
    pub(crate) status: String,
    pub(crate) generation_id: String,
    pub(crate) loop_count: u64,
    /// Canonical Stop key `(session_id, generation_id, loop_count)`.
    pub(crate) stop_key: String,
    /// Validated snapshot evidence; `None` on every degraded path.
    pub(crate) snapshot: Option<CursorSnapshotEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CursorSnapshotEvidence {
    /// Hex SHA-256 of the complete stable snapshot bytes.
    pub(crate) snapshot_hash: String,
    pub(crate) snapshot_byte_len: u64,
    /// Total physical record count (messages and boundary records).
    pub(crate) record_count: u64,
    /// Usable, redacted user/assistant messages in physical record order.
    /// Ordinals are physical JSONL positions and may have holes.
    pub(crate) messages: Vec<CursorSnapshotMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CursorSnapshotMessage {
    pub(crate) ordinal: u64,
    pub(crate) role: String,
    pub(crate) text: String,
}

/// Result of the Stop-time snapshot attempt. Degradation never drops the
/// Stop: the caller records the Stop with the degraded marker and previously
/// captured payload evidence.
#[derive(Debug)]
pub(crate) enum CursorSnapshotOutcome {
    Full {
        evidence: CursorSnapshotEvidence,
        last_assistant_message: Option<String>,
    },
    Degraded {
        reason: &'static str,
    },
}

impl CursorSnapshotOutcome {
    pub(crate) fn reason_code(&self) -> Option<&'static str> {
        match self {
            CursorSnapshotOutcome::Full { .. } => None,
            CursorSnapshotOutcome::Degraded { reason } => Some(reason),
        }
    }
}

pub(crate) const REASON_PATH_ABSENT: &str = "path_absent";
pub(crate) const REASON_PATH_BLANK: &str = "path_blank";
pub(crate) const REASON_PATH_UNTRUSTED: &str = "path_untrusted";
pub(crate) const REASON_READ_FAILED: &str = "read_failed";
pub(crate) const REASON_SNAPSHOT_CHANGED: &str = "snapshot_changed";
pub(crate) const REASON_SNAPSHOT_TOO_LARGE: &str = "snapshot_too_large";
pub(crate) const REASON_FORMAT_INVALID: &str = "format_invalid";
pub(crate) const REASON_TRANSCRIPT_EMPTY: &str = "transcript_empty";

/// Captures and fully validates the Stop transcript snapshot, or returns an
/// explicit degradation reason. Never partial: a failed validation yields
/// zero transcript-derived evidence.
pub(crate) fn capture_stop_snapshot(transcript_path: Option<&str>) -> CursorSnapshotOutcome {
    let Some(raw_path) = transcript_path else {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_PATH_ABSENT,
        };
    };
    if raw_path.trim().is_empty() {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_PATH_BLANK,
        };
    }
    if !cursor_transcript_path_is_trusted(raw_path) {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_PATH_UNTRUSTED,
        };
    }
    let metadata = match std::fs::symlink_metadata(raw_path) {
        Ok(metadata) => metadata,
        Err(_) => {
            return CursorSnapshotOutcome::Degraded {
                reason: REASON_READ_FAILED,
            }
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_PATH_UNTRUSTED,
        };
    }
    if metadata.len() > CURSOR_TRANSCRIPT_MAX_BYTES {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_SNAPSHOT_TOO_LARGE,
        };
    }
    let bytes = match std::fs::read(raw_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return CursorSnapshotOutcome::Degraded {
                reason: REASON_READ_FAILED,
            }
        }
    };
    if bytes.len() as u64 > CURSOR_TRANSCRIPT_MAX_BYTES {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_SNAPSHOT_TOO_LARGE,
        };
    }
    if bytes.len() as u64 != metadata.len() {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_SNAPSHOT_CHANGED,
        };
    }
    let transcript = match parse_cursor_transcript(&bytes) {
        Ok(transcript) => transcript,
        Err(error) => {
            crate::log::error(
                "summarize",
                &format!(
                    "cursor transcript full validation failed; degrading to payload-only: {error:#}"
                ),
            );
            return CursorSnapshotOutcome::Degraded {
                reason: REASON_FORMAT_INVALID,
            };
        }
    };
    build_full_outcome(&transcript)
}

fn build_full_outcome(transcript: &ValidatedCursorTranscript) -> CursorSnapshotOutcome {
    let messages: Vec<CursorSnapshotMessage> = transcript
        .usable_messages()
        .map(|(ordinal, role, text)| CursorSnapshotMessage {
            ordinal,
            role: role.as_str().to_string(),
            text: crate::adapter::common::redact_sensitive_text(text),
        })
        .collect();
    if messages.is_empty() {
        return CursorSnapshotOutcome::Degraded {
            reason: REASON_TRANSCRIPT_EMPTY,
        };
    }
    let last_assistant_message = transcript
        .last_assistant_text()
        .map(crate::adapter::common::redact_sensitive_text);
    CursorSnapshotOutcome::Full {
        evidence: CursorSnapshotEvidence {
            snapshot_hash: transcript.snapshot_hash.clone(),
            snapshot_byte_len: transcript.snapshot_byte_len,
            record_count: transcript.records.len() as u64,
            messages,
        },
        last_assistant_message,
    }
}

/// Trusted-path proposal pending #822's frozen trusted-root evidence: the
/// path must be an absolute Unix path without backslashes, UNC-like `//`
/// prefixes, drive letters, or `.`/`..` traversal components. Symlinks and
/// non-regular files are rejected separately against `symlink_metadata`.
fn cursor_transcript_path_is_trusted(raw_path: &str) -> bool {
    if raw_path.contains('\\') || !raw_path.starts_with('/') || raw_path.starts_with("//") {
        return false;
    }
    let mut chars = raw_path.chars();
    let _slash = chars.next();
    if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() && second == ':' {
            return false;
        }
    }
    std::path::Path::new(raw_path)
        .components()
        .all(|component| {
            matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_transcript(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "remem-gh825-{name}-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, contents).expect("write cursor transcript fixture");
        path
    }

    const VALID: &str = concat!(
        "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"question\"}]}}\n",
        "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"answer\"}]}}\n",
        "{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
    );

    #[test]
    fn missing_and_blank_paths_degrade_without_dropping_the_stop() {
        assert_eq!(
            capture_stop_snapshot(None).reason_code(),
            Some(REASON_PATH_ABSENT)
        );
        assert_eq!(
            capture_stop_snapshot(Some("   ")).reason_code(),
            Some(REASON_PATH_BLANK)
        );
    }

    #[test]
    fn untrusted_path_shapes_are_never_read() {
        for path in [
            "relative/transcript.jsonl",
            "/tmp/../etc/passwd",
            "//server/share/t.jsonl",
            "/c:/Users/t.jsonl",
            "C:\\Users\\t.jsonl",
        ] {
            assert_eq!(
                capture_stop_snapshot(Some(path)).reason_code(),
                Some(REASON_PATH_UNTRUSTED),
                "path must be untrusted: {path}"
            );
        }
    }

    #[test]
    fn missing_file_and_symlink_degrade_explicitly() {
        assert_eq!(
            capture_stop_snapshot(Some("/nonexistent/remem-gh825/transcript.jsonl")).reason_code(),
            Some(REASON_READ_FAILED)
        );
        let target = temp_transcript("symlink-target", VALID);
        let link = std::env::temp_dir().join(format!(
            "remem-gh825-symlink-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::os::unix::fs::symlink(&target, &link).expect("create symlink fixture");
        assert_eq!(
            capture_stop_snapshot(Some(&link.to_string_lossy())).reason_code(),
            Some(REASON_PATH_UNTRUSTED)
        );
        std::fs::remove_file(&link).expect("remove symlink");
        std::fs::remove_file(&target).expect("remove target");
    }

    #[test]
    fn oversized_corrupt_and_empty_snapshots_degrade_with_stable_reasons() {
        let oversized = temp_transcript(
            "oversized",
            &"{\"type\":\"turn_ended\",\"status\":\"success\"}\n"
                .repeat((CURSOR_TRANSCRIPT_MAX_BYTES as usize / 40) + 2),
        );
        assert_eq!(
            capture_stop_snapshot(Some(&oversized.to_string_lossy())).reason_code(),
            Some(REASON_SNAPSHOT_TOO_LARGE)
        );
        std::fs::remove_file(&oversized).expect("remove oversized");

        let corrupt = temp_transcript("corrupt", "{not-json\n");
        assert_eq!(
            capture_stop_snapshot(Some(&corrupt.to_string_lossy())).reason_code(),
            Some(REASON_FORMAT_INVALID)
        );
        std::fs::remove_file(&corrupt).expect("remove corrupt");

        // Valid grammar with zero usable user/assistant evidence.
        let empty = temp_transcript(
            "empty",
            "{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
        );
        assert_eq!(
            capture_stop_snapshot(Some(&empty.to_string_lossy())).reason_code(),
            Some(REASON_TRANSCRIPT_EMPTY)
        );
        std::fs::remove_file(&empty).expect("remove empty");
    }

    #[test]
    fn full_snapshot_keeps_physical_ordinals_and_redacted_text() {
        let path = temp_transcript("full", VALID);
        let outcome = capture_stop_snapshot(Some(&path.to_string_lossy()));
        let CursorSnapshotOutcome::Full {
            evidence,
            last_assistant_message,
        } = outcome
        else {
            panic!("valid transcript must produce a full snapshot");
        };
        assert_eq!(evidence.record_count, 3);
        assert_eq!(evidence.messages.len(), 2);
        assert_eq!(evidence.messages[0].ordinal, 0);
        assert_eq!(evidence.messages[1].ordinal, 1);
        assert_eq!(evidence.messages[1].role, "assistant");
        assert_eq!(last_assistant_message.as_deref(), Some("answer"));
        assert_eq!(evidence.snapshot_byte_len, VALID.len() as u64);
        assert_eq!(evidence.snapshot_hash.len(), 64);
        std::fs::remove_file(&path).expect("remove fixture");
    }
}
