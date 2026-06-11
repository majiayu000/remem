use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

use crate::adapter::{EventSummary, ParsedHookEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureSpillRecord {
    version: u32,
    event_id: String,
    host: String,
    event: ParsedHookEvent,
    summary: EventSummary,
    db_error: String,
    created_at_epoch: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct CaptureSpillRecordCompat {
    version: u32,
    event_id: Option<String>,
    host: String,
    event: ParsedHookEvent,
    summary: EventSummary,
    db_error: String,
    created_at_epoch: i64,
}

impl From<CaptureSpillRecordCompat> for CaptureSpillRecord {
    fn from(record: CaptureSpillRecordCompat) -> Self {
        let content = legacy_spill_event_content(&record.event, &record.summary);
        Self {
            version: record.version,
            event_id: record
                .event_id
                .unwrap_or_else(|| crate::db::unique_capture_event_id("tool_result", &content)),
            host: record.host,
            event: record.event,
            summary: record.summary,
            db_error: record.db_error,
            created_at_epoch: record.created_at_epoch,
        }
    }
}

pub(super) fn record_capture_drop_lossy(
    host: Option<&str>,
    event: Option<&ParsedHookEvent>,
    reason: &str,
    detail: Option<&str>,
) {
    let Ok(conn) = crate::db::open_db() else {
        crate::log::warn(
            "observe",
            &format!("capture drop could not be recorded: reason={reason}"),
        );
        return;
    };
    let result = crate::db::record_capture_drop(
        &conn,
        &crate::db::CaptureDropInput {
            host,
            session_id: event.map(|event| event.session_id.as_str()),
            project: event.map(|event| event.project.as_str()),
            tool_name: event.map(|event| event.tool_name.as_str()),
            reason,
            detail,
            spill_path: None,
            recovered_event_id: None,
        },
    );
    if let Err(error) = result {
        crate::log::warn(
            "observe",
            &format!("capture drop ledger write failed: {error}"),
        );
    }
}

pub(super) fn spill_capture_event(
    host: &str,
    event_id: &str,
    event: &ParsedHookEvent,
    summary: &EventSummary,
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    let path = spill_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create capture spill dir {}", parent.display()))?;
    }
    let record = CaptureSpillRecord {
        version: 1,
        event_id: event_id.to_string(),
        host: host.to_string(),
        event: sanitize_event(event),
        summary: sanitize_summary(summary),
        db_error: crate::db::truncate_str(
            &crate::db::capture::redact_capture_content(&db_error.to_string()),
            1000,
        )
        .to_string(),
        created_at_epoch: chrono::Utc::now().timestamp(),
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open capture spill {}", path.display()))?;
    serde_json::to_writer(&mut file, &record)?;
    file.write_all(b"\n")?;
    Ok(path)
}

pub(super) fn replay_spilled_capture_events(conn: &Connection) -> Result<usize> {
    let path = spill_path();
    if !path.exists() {
        return Ok(0);
    }

    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut replayed = 0;
    let failed_path = failed_spill_path();
    if failed_path.exists() {
        std::fs::remove_file(&failed_path)
            .with_context(|| format!("remove stale {}", failed_path.display()))?;
    }

    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        match parse_spill_record(line) {
            Ok(record) => match super::hook::record_observed_event_with_id(
                conn,
                &record.host,
                &record.event_id,
                &record.event,
                &record.summary,
            ) {
                Ok(event_id) => {
                    crate::db::record_capture_drop(
                        conn,
                        &crate::db::CaptureDropInput {
                            host: Some(&record.host),
                            session_id: Some(&record.event.session_id),
                            project: Some(&record.event.project),
                            tool_name: Some(&record.event.tool_name),
                            reason: "db_open_failed",
                            detail: Some(&record.db_error),
                            spill_path: Some(&path.display().to_string()),
                            recovered_event_id: Some(event_id),
                        },
                    )?;
                    replayed += 1;
                }
                Err(error) => append_failed_spill(&failed_path, line, &error)?,
            },
            Err(error) => append_failed_spill(&failed_path, line, &error)?,
        }
    }

    std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    if failed_path.exists() {
        std::fs::rename(&failed_path, &path).with_context(|| {
            format!(
                "move unreplayed capture spill {} to {}",
                failed_path.display(),
                path.display()
            )
        })?;
    }

    if replayed > 0 {
        crate::log::info(
            "observe",
            &format!("replayed {replayed} spilled capture event(s)"),
        );
    }
    Ok(replayed)
}

fn append_failed_spill(path: &PathBuf, line: &str, error: &anyhow::Error) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create failed capture spill dir {}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open failed capture spill {}", path.display()))?;
    writeln!(file, "{line}")?;
    crate::log::warn("observe", &format!("capture spill replay failed: {error}"));
    Ok(())
}

fn sanitize_event(event: &ParsedHookEvent) -> ParsedHookEvent {
    ParsedHookEvent {
        session_id: event.session_id.clone(),
        cwd: event.cwd.clone(),
        project: event.project.clone(),
        tool_name: event.tool_name.clone(),
        tool_input: event
            .tool_input
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
        tool_response: event
            .tool_response
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
    }
}

fn sanitize_summary(summary: &EventSummary) -> EventSummary {
    EventSummary {
        event_type: summary.event_type.clone(),
        summary: crate::db::capture::redact_capture_content(&summary.summary),
        detail: summary
            .detail
            .as_ref()
            .map(|detail| crate::db::capture::redact_capture_content(detail)),
        files_json: summary.files_json.clone(),
        exit_code: summary.exit_code,
    }
}

fn parse_spill_record(line: &str) -> Result<CaptureSpillRecord> {
    let record: CaptureSpillRecordCompat = serde_json::from_str(line)?;
    Ok(record.into())
}

fn legacy_spill_event_content(event: &ParsedHookEvent, summary: &EventSummary) -> String {
    serde_json::json!({
        "summary": &summary.summary,
        "event_type": &summary.event_type,
        "detail": summary.detail.as_deref(),
        "files": summary.files_json.as_deref(),
        "exit_code": summary.exit_code,
        "tool_name": &event.tool_name,
        "tool_input": event
            .tool_input
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
        "tool_response": event
            .tool_response
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
    })
    .to_string()
}

fn spill_path() -> PathBuf {
    crate::db::data_dir().join("capture-spill.jsonl")
}

fn failed_spill_path() -> PathBuf {
    crate::db::data_dir().join("capture-spill.failed.jsonl")
}

#[cfg(test)]
mod tests {
    use crate::adapter::{EventSummary, ParsedHookEvent};
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::{replay_spilled_capture_events, spill_capture_event};

    #[test]
    fn replay_spilled_capture_event_records_capture_and_drop_ledger() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-replay");
        let err = anyhow::anyhow!("database is locked");
        let event = ParsedHookEvent {
            session_id: "session-spill".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Edit".to_string(),
            tool_input: Some(serde_json::json!({"file_path": "/tmp/remem/src/lib.rs"})),
            tool_response: Some(serde_json::json!({"ok": true})),
        };
        let summary = EventSummary {
            event_type: "file_edit".to_string(),
            summary: "Edited src/lib.rs".to_string(),
            detail: None,
            files_json: Some("[\"src/lib.rs\"]".to_string()),
            exit_code: None,
        };
        let content = serde_json::json!({
            "summary": summary.summary,
            "tool_name": event.tool_name,
            "tool_input": event.tool_input,
            "tool_response": event.tool_response,
        })
        .to_string();
        let event_id = db::unique_capture_event_id("tool_result", &content);
        spill_capture_event("codex-cli", &event_id, &event, &summary, &err)?;
        let conn = db::open_db()?;

        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 1);
        let captured: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let drops: i64 = conn.query_row("SELECT COUNT(*) FROM capture_drop_events", [], |row| {
            row.get(0)
        })?;
        assert_eq!(captured, 1);
        assert_eq!(drops, 1);
        Ok(())
    }

    #[test]
    fn replay_identical_spills_preserves_distinct_captured_events() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-identical-replay");
        let err = anyhow::anyhow!("database is locked");
        let event = ParsedHookEvent {
            session_id: "session-identical-spill".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Edit".to_string(),
            tool_input: Some(serde_json::json!({"file_path": "/tmp/remem/src/lib.rs"})),
            tool_response: Some(serde_json::json!({"ok": true})),
        };
        let summary = EventSummary {
            event_type: "file_edit".to_string(),
            summary: "Edited src/lib.rs".to_string(),
            detail: None,
            files_json: Some("[\"src/lib.rs\"]".to_string()),
            exit_code: None,
        };

        spill_capture_event(
            "codex-cli",
            "tool_result-identical-a",
            &event,
            &summary,
            &err,
        )?;
        spill_capture_event(
            "codex-cli",
            "tool_result-identical-b",
            &event,
            &summary,
            &err,
        )?;
        let conn = db::open_db()?;

        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 2);
        let captured: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        assert_eq!(captured, 2);
        Ok(())
    }

    #[test]
    fn replay_legacy_spill_without_event_id() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-legacy-replay");
        let legacy = serde_json::json!({
            "version": 1,
            "host": "codex-cli",
            "event": {
                "session_id": "session-legacy-spill",
                "cwd": "/tmp/remem",
                "project": "/tmp/remem",
                "tool_name": "Edit",
                "tool_input": {"file_path": "/tmp/remem/src/lib.rs"},
                "tool_response": {"ok": true}
            },
            "summary": {
                "event_type": "file_edit",
                "summary": "Edited src/lib.rs",
                "detail": null,
                "files_json": "[\"src/lib.rs\"]",
                "exit_code": null
            },
            "db_error": "database is locked",
            "created_at_epoch": 1700000000
        });
        let path = crate::db::data_dir().join("capture-spill.jsonl");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, format!("{legacy}\n"))?;
        let conn = db::open_db()?;

        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 1);
        let captured: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-legacy-spill'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(captured, 1);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn spill_redacts_summary_and_database_error() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-redacts");
        let err = anyhow::anyhow!("database token=github_pat_secret");
        let event = ParsedHookEvent {
            session_id: "session-spill-redact".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Bash".to_string(),
            tool_input: Some(serde_json::json!({
                "command": "curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456'"
            })),
            tool_response: Some(serde_json::json!({"stderr": "password=hunter2"})),
        };
        let summary = EventSummary {
            event_type: "bash".to_string(),
            summary: "Run curl with ghp_abcdefghijklmnopqrstuvwxyz123456".to_string(),
            detail: Some("password=hunter2".to_string()),
            files_json: None,
            exit_code: Some(1),
        };

        spill_capture_event("codex-cli", "tool_result-redact", &event, &summary, &err)?;
        let stored = std::fs::read_to_string(crate::db::data_dir().join("capture-spill.jsonl"))?;

        assert!(stored.contains("[REDACTED]"));
        assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!stored.contains("hunter2"));
        assert!(!stored.contains("github_pat_secret"));
        Ok(())
    }
}
