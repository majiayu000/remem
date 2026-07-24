use anyhow::Result;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Cursor `remem summarize` entrypoint (GH-823 SP823-T5 wiring into GH-825's
/// verified Cursor transcript reader).
///
/// Flow: bounded stdin read → full Cursor Stop validation (approved status
/// set, non-empty `generation_id`, normalized `loop_count`, canonical Stop
/// key) → Stop-time transcript snapshot with full validation or explicit
/// machine-readable degradation → prepared summarize payload with host
/// `cursor`. The prepared payload never carries a `transcript_path`, so the
/// Claude/Codex raw transcript parser and the path-based drain are
/// unreachable from a Cursor Stop; the worker never reopens the path.
pub async fn summarize_cursor() -> Result<()> {
    let bytes = crate::cursor_hook::input::read_bounded_hook_stdin(&mut std::io::stdin().lock())?;
    summarize_cursor_bytes(&bytes).await
}

pub async fn summarize_cursor_bytes(bytes: &[u8]) -> Result<()> {
    let stop = crate::cursor_hook::stop::parse_stop_event(bytes)?;
    let outcome = crate::session_rollup::cursor_snapshot::capture_stop_snapshot(
        stop.transcript_path.as_deref(),
    );
    if let Some(reason) = outcome.reason_code() {
        crate::log::error(
            "summarize",
            &format!(
                "cursor transcript degraded to payload-only: reason={reason} stop_key={} locator={}",
                stop.canonical_stop_key(),
                degraded_path_locator(stop.transcript_path.as_deref()),
            ),
        );
    }
    let (marker, last_assistant_message) = cursor_capture_marker(&stop, outcome);
    let payload = build_cursor_summary_payload(&stop, &marker, last_assistant_message.as_deref())?;
    super::summary_job::summarize_cursor_prepared_input(&payload).await?;
    if marker.fidelity == crate::session_rollup::cursor_snapshot::CURSOR_FIDELITY_DEGRADED {
        record_cursor_degraded_drop(&stop, &marker);
    }
    Ok(())
}

fn cursor_capture_marker(
    stop: &crate::cursor_hook::stop::CursorStopEvent,
    outcome: crate::session_rollup::cursor_snapshot::CursorSnapshotOutcome,
) -> (
    crate::session_rollup::cursor_snapshot::CursorCaptureMarker,
    Option<String>,
) {
    use crate::session_rollup::cursor_snapshot::{
        CursorCaptureMarker, CursorSnapshotOutcome, CURSOR_FIDELITY_DEGRADED, CURSOR_FIDELITY_FULL,
    };
    let (fidelity, reason_code, snapshot, last_assistant_message) = match outcome {
        CursorSnapshotOutcome::Full {
            evidence,
            last_assistant_message,
        } => (
            CURSOR_FIDELITY_FULL,
            None,
            Some(evidence),
            last_assistant_message,
        ),
        CursorSnapshotOutcome::Degraded { reason } => (
            CURSOR_FIDELITY_DEGRADED,
            Some(reason.to_string()),
            None,
            None,
        ),
    };
    (
        CursorCaptureMarker {
            fidelity: fidelity.to_string(),
            reason_code,
            status: stop.status.as_str().to_string(),
            generation_id: stop.generation_id.clone(),
            loop_count: stop.loop_count,
            stop_key: stop.canonical_stop_key(),
            snapshot,
        },
        last_assistant_message,
    )
}

/// Builds the prepared Cursor summarize payload. Deliberately excludes
/// `transcript_path` and `transcript_byte_len`: the snapshot IR inside
/// `cursor_capture` is the only transcript-derived evidence downstream.
fn build_cursor_summary_payload(
    stop: &crate::cursor_hook::stop::CursorStopEvent,
    marker: &crate::session_rollup::cursor_snapshot::CursorCaptureMarker,
    last_assistant_message: Option<&str>,
) -> Result<String> {
    let mut payload = serde_json::json!({
        "session_id": stop.session_id,
        "cwd": stop.workspace_root,
        "cursor_capture": marker,
    });
    if let Some(message) = last_assistant_message {
        payload["last_assistant_message"] = serde_json::Value::String(message.to_string());
    }
    Ok(serde_json::to_string(&payload)?)
}

/// Sanitized transcript locator for degraded-path logs and diagnostics:
/// never the raw path, only a 16-hex-char SHA-256 prefix (or a stable
/// keyword when no path string exists).
fn degraded_path_locator(transcript_path: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    match transcript_path {
        None => "path-absent".to_string(),
        Some(path) if path.trim().is_empty() => "path-blank".to_string(),
        Some(path) => {
            let digest = format!("{:x}", Sha256::digest(path.as_bytes()));
            format!("sha256:{}", &digest[..16])
        }
    }
}

/// Best-effort degraded-transcript diagnostic in the existing
/// `capture_drop_events` table (`cursor_transcript_<reason>`). The Stop and
/// its payload evidence are already durable at this point, so a failure here
/// is logged at error level but does not fail the hook.
fn record_cursor_degraded_drop(
    stop: &crate::cursor_hook::stop::CursorStopEvent,
    marker: &crate::session_rollup::cursor_snapshot::CursorCaptureMarker,
) {
    let Some(reason_code) = marker.reason_code.as_deref() else {
        return;
    };
    let reason = format!("cursor_transcript_{reason_code}");
    let detail = format!(
        "stop_key={} status={} locator={}",
        marker.stop_key,
        marker.status,
        degraded_path_locator(stop.transcript_path.as_deref()),
    );
    let result = crate::db::open_db_for_hook().and_then(|conn| {
        crate::db::record_capture_drop(
            &conn,
            &crate::db::CaptureDropInput {
                host: Some(crate::cursor_hook::CURSOR_HOST),
                session_id: Some(&stop.session_id),
                project: Some(&stop.workspace_root),
                tool_name: None,
                reason: &reason,
                detail: Some(&detail),
                spill_path: None,
                recovered_event_id: None,
            },
        )
    });
    if let Err(error) = result {
        crate::log::error(
            "summarize",
            &format!(
                "cursor degraded-transcript diagnostic write failed (stop and payload evidence remain durable): {error:#}"
            ),
        );
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    fn stop_event(transcript_path: Option<&str>) -> crate::cursor_hook::stop::CursorStopEvent {
        let payload = serde_json::json!({
            "conversation_id": "sess-cursor-1",
            "generation_id": "gen-1",
            "status": "completed",
            "loop_count": 0,
            "session_id": "sess-cursor-1",
            "hook_event_name": "stop",
            "workspace_roots": ["/tmp/remem-cursor"],
            "transcript_path": transcript_path,
        });
        crate::cursor_hook::stop::parse_stop_event(payload.to_string().as_bytes())
            .expect("stop fixture validates")
    }

    #[test]
    fn full_cursor_payload_embeds_ir_and_never_a_transcript_path() {
        let path = std::env::temp_dir().join(format!(
            "remem-gh825-payload-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &path,
            concat!(
                "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ask\"}]}}\n",
                "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"answer\"}]}}\n",
                "{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
            ),
        )
        .expect("write cursor transcript fixture");
        let stop = stop_event(Some(&path.to_string_lossy()));

        let outcome = crate::session_rollup::cursor_snapshot::capture_stop_snapshot(
            stop.transcript_path.as_deref(),
        );
        let (marker, last_assistant_message) = cursor_capture_marker(&stop, outcome);
        let payload =
            build_cursor_summary_payload(&stop, &marker, last_assistant_message.as_deref())
                .expect("payload builds");
        std::fs::remove_file(&path).expect("remove fixture");

        let value: serde_json::Value = serde_json::from_str(&payload).expect("payload is JSON");
        let object = value.as_object().expect("payload object");
        assert!(
            !object.contains_key("transcript_path"),
            "cursor payload must never carry a transcript path"
        );
        assert!(!object.contains_key("transcript_byte_len"));
        assert_eq!(object["session_id"], "sess-cursor-1");
        assert_eq!(object["cwd"], "/tmp/remem-cursor");
        assert_eq!(object["last_assistant_message"], "answer");
        let capture = object["cursor_capture"].as_object().expect("marker");
        assert_eq!(capture["fidelity"], "full");
        assert_eq!(capture["reason_code"], serde_json::Value::Null);
        assert_eq!(capture["status"], "completed");
        assert_eq!(capture["stop_key"], "sess-cursor-1:gen-1:0");
        let snapshot = capture["snapshot"].as_object().expect("snapshot");
        assert_eq!(snapshot["record_count"], 3);
        let messages = snapshot["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["ordinal"], 0);
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[test]
    fn degraded_cursor_payload_carries_a_machine_readable_marker() {
        let stop = stop_event(None);
        let outcome = crate::session_rollup::cursor_snapshot::capture_stop_snapshot(
            stop.transcript_path.as_deref(),
        );
        let (marker, last_assistant_message) = cursor_capture_marker(&stop, outcome);
        assert!(last_assistant_message.is_none());
        let payload = build_cursor_summary_payload(&stop, &marker, None).expect("payload builds");
        let value: serde_json::Value = serde_json::from_str(&payload).expect("payload is JSON");
        let capture = value["cursor_capture"].as_object().expect("marker");
        assert_eq!(capture["fidelity"], "degraded");
        assert_eq!(capture["reason_code"], "path_absent");
        assert_eq!(capture["snapshot"], serde_json::Value::Null);
        assert!(!value
            .as_object()
            .unwrap()
            .contains_key("last_assistant_message"));
    }

    #[test]
    fn degraded_path_locator_never_echoes_the_raw_path() {
        let locator = degraded_path_locator(Some("/private/home/user/secret.jsonl"));
        assert!(locator.starts_with("sha256:"));
        assert_eq!(locator.len(), "sha256:".len() + 16);
        assert!(!locator.contains("secret"));
        assert_eq!(degraded_path_locator(None), "path-absent");
        assert_eq!(degraded_path_locator(Some("  ")), "path-blank");
    }
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
