use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::adapter::{EventSummary, ParsedHookEvent};

pub(super) const SPILL_REASON_DB_OPEN_FAILED: &str = "db_open_failed";
pub(super) const SPILL_REASON_CAPTURE_PERSISTENCE_FAILED: &str = "capture_persistence_failed";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureSpillRecord {
    version: u32,
    event_id: String,
    host: String,
    event: ParsedHookEvent,
    summary: EventSummary,
    failure_reason: String,
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
    failure_reason: Option<String>,
    db_error: String,
    created_at_epoch: i64,
}

impl CaptureSpillRecordCompat {
    fn into_record(self, fallback_event_id: String) -> CaptureSpillRecord {
        CaptureSpillRecord {
            version: self.version,
            event_id: self.event_id.unwrap_or(fallback_event_id),
            host: self.host,
            event: sanitize_event(&self.event),
            summary: sanitize_summary(&self.summary),
            failure_reason: self
                .failure_reason
                .unwrap_or_else(|| SPILL_REASON_DB_OPEN_FAILED.to_string()),
            db_error: crate::db::truncate_str(
                &crate::db::capture::redact_capture_content(&self.db_error),
                1000,
            )
            .to_string(),
            created_at_epoch: self.created_at_epoch,
        }
    }
}

pub(super) fn record_capture_drop_lossy(
    host: Option<&str>,
    event: Option<&ParsedHookEvent>,
    reason: &str,
    detail: Option<&str>,
) {
    let Ok(conn) = crate::db::open_db_for_hook() else {
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
    failure_reason: &str,
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
        failure_reason: failure_reason.to_string(),
        db_error: crate::db::truncate_str(
            &crate::db::capture::redact_capture_content(&db_error.to_string()),
            1000,
        )
        .to_string(),
        created_at_epoch: chrono::Utc::now().timestamp(),
    };
    append_spill_record(&path, &record)?;
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

    for (line_index, line) in contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        match parse_spill_record(line, line_index) {
            Ok(record) => match replay_spill_record(conn, &path, &record) {
                Ok(true) => replayed += 1,
                Ok(false) => {}
                Err(error) => append_failed_spill_record(&failed_path, &record, &error)?,
            },
            Err(error) => append_failed_spill_line(&failed_path, line, &error)?,
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

fn replay_spill_record(
    conn: &Connection,
    spill_path: &Path,
    record: &CaptureSpillRecord,
) -> Result<bool> {
    if recovered_spill_exists(conn, record)? {
        return Ok(false);
    }

    let tx = conn
        .unchecked_transaction()
        .context("start capture spill replay transaction")?;
    let event_id = super::hook::record_observed_event_with_id(
        &tx,
        &record.host,
        &record.event_id,
        &record.event,
        &record.summary,
    )?;
    let spill_path = spill_path.display().to_string();
    let drop_input = crate::db::CaptureDropInput {
        host: Some(&record.host),
        session_id: Some(&record.event.session_id),
        project: Some(&record.event.project),
        tool_name: Some(&record.event.tool_name),
        reason: &record.failure_reason,
        detail: Some(&record.db_error),
        spill_path: Some(&spill_path),
        recovered_event_id: Some(event_id),
    };
    if !crate::db::mark_capture_spill_recovered(&tx, &drop_input, event_id)? {
        crate::db::record_capture_drop(&tx, &drop_input)?;
    }
    tx.commit()
        .context("commit capture spill replay transaction")?;
    Ok(true)
}

fn append_failed_spill_line(path: &Path, line: &str, error: &anyhow::Error) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create failed capture spill dir {}", parent.display()))?;
    }
    let mut file = spill_open_options()
        .open(path)
        .with_context(|| format!("open failed capture spill {}", path.display()))?;
    writeln!(file, "{line}")?;
    crate::log::warn("observe", &format!("capture spill replay failed: {error}"));
    Ok(())
}

fn append_failed_spill_record(
    path: &Path,
    record: &CaptureSpillRecord,
    error: &anyhow::Error,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create failed capture spill dir {}", parent.display()))?;
    }
    append_spill_record(path, record)?;
    crate::log::warn("observe", &format!("capture spill replay failed: {error}"));
    Ok(())
}

fn append_spill_record(path: &Path, record: &CaptureSpillRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create capture spill dir {}", parent.display()))?;
    }
    let line = crate::db::spill_crypto::encode_json_line(record)?;
    let mut file = spill_open_options()
        .open(path)
        .with_context(|| format!("open capture spill {}", path.display()))?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn spill_open_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
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

fn parse_spill_record(line: &str, line_index: usize) -> Result<CaptureSpillRecord> {
    let record: CaptureSpillRecordCompat = crate::db::spill_crypto::decode_json_line(line)?;
    Ok(record.into_record(legacy_spill_event_id(line, line_index)))
}

fn recovered_spill_exists(conn: &Connection, record: &CaptureSpillRecord) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1
             FROM captured_events captured
             JOIN hosts ON hosts.id = captured.host_id
             JOIN capture_drop_events drop_event
               ON drop_event.recovered_event_id = captured.id
             WHERE hosts.name = ?1
               AND captured.session_id = ?2
               AND captured.event_id = ?3
               AND drop_event.reason = ?4
             LIMIT 1",
            rusqlite::params![
                &record.host,
                &record.event.session_id,
                &record.event_id,
                &record.failure_reason
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

fn legacy_spill_event_id(line: &str, line_index: usize) -> String {
    format!(
        "tool_result-legacy-spill-{}-{:016x}",
        line_index + 1,
        crate::db::deterministic_hash(line.as_bytes())
    )
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

    use super::{replay_spilled_capture_events, spill_capture_event, SPILL_REASON_DB_OPEN_FAILED};

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
        spill_capture_event(
            "codex-cli",
            &event_id,
            &event,
            &summary,
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        let conn = db::open_db()?;

        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 1);
        let captured: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let drops: i64 = conn.query_row("SELECT COUNT(*) FROM capture_drop_events", [], |row| {
            row.get(0)
        })?;
        let drop_reason: String =
            conn.query_row("SELECT reason FROM capture_drop_events", [], |row| {
                row.get(0)
            })?;
        assert_eq!(captured, 1);
        assert_eq!(drops, 1);
        assert_eq!(drop_reason, SPILL_REASON_DB_OPEN_FAILED);
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
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        spill_capture_event(
            "codex-cli",
            "tool_result-identical-b",
            &event,
            &summary,
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        let conn = db::open_db()?;

        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 2);
        let captured: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let events: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        assert_eq!(captured, 2);
        assert_eq!(events, 2);
        Ok(())
    }

    #[test]
    fn replay_identical_spill_retry_appends_partial_second_legacy_event() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-identical-partial-retry");
        let err = anyhow::anyhow!("database is locked");
        let event = ParsedHookEvent {
            session_id: "session-identical-partial".to_string(),
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
            "tool_result-identical-partial-a",
            &event,
            &summary,
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        spill_capture_event(
            "codex-cli",
            "tool_result-identical-partial-b",
            &event,
            &summary,
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        let conn = db::open_db()?;
        conn.execute_batch(
            "CREATE TRIGGER fail_second_legacy_event
             BEFORE INSERT ON events
             WHEN (SELECT COUNT(*) FROM events WHERE session_id = 'session-identical-partial') >= 1
             BEGIN
                 SELECT RAISE(FAIL, 'legacy events blocked');
             END;",
        )?;

        assert_eq!(replay_spilled_capture_events(&conn)?, 1);
        let partial_captured: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-identical-partial'",
            [],
            |row| row.get(0),
        )?;
        let partial_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'session-identical-partial'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(partial_captured, 1);
        assert_eq!(partial_events, 1);

        conn.execute_batch("DROP TRIGGER fail_second_legacy_event;")?;
        assert_eq!(replay_spilled_capture_events(&conn)?, 1);
        let replayed_captured: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-identical-partial'",
            [],
            |row| row.get(0),
        )?;
        let replayed_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'session-identical-partial'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(replayed_captured, 2);
        assert_eq!(replayed_events, 2);
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
                "summary": "Edited src/lib.rs with ghp_abcdefghijklmnopqrstuvwxyz123456",
                "detail": "password=hunter2",
                "files_json": "[\"src/lib.rs\"]",
                "exit_code": null
            },
            "db_error": "database token=github_pat_secret",
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
        let replayed_event: (String, String) = conn.query_row(
            "SELECT summary, detail FROM events WHERE session_id = 'session-legacy-spill'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let drop_detail: String = conn.query_row(
            "SELECT detail FROM capture_drop_events WHERE session_id = 'session-legacy-spill'",
            [],
            |row| row.get(0),
        )?;
        assert!(replayed_event.0.contains("[REDACTED]"));
        assert!(!replayed_event
            .0
            .contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!replayed_event.1.contains("hunter2"));
        assert!(!drop_detail.contains("github_pat_secret"));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn replay_legacy_spill_preserves_synthesized_id_after_partial_failure() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-legacy-stable-id");
        let legacy = serde_json::json!({
            "version": 1,
            "host": "codex-cli",
            "event": {
                "session_id": "session-legacy-stable",
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
        conn.execute_batch(
            "CREATE TRIGGER fail_legacy_events_insert
             BEFORE INSERT ON events
             BEGIN
                 SELECT RAISE(FAIL, 'legacy events blocked');
             END;",
        )?;

        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 0);
        let partial_captures: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-legacy-stable'",
            [],
            |row| row.get(0),
        )?;
        let partial_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'session-legacy-stable'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(partial_captures, 0);
        assert_eq!(partial_events, 0);
        let failed_spill = std::fs::read_to_string(&path)?;
        assert!(failed_spill.contains(r#""event_id":"tool_result-legacy-spill-1-"#));

        conn.execute_batch("DROP TRIGGER fail_legacy_events_insert;")?;
        let replayed = replay_spilled_capture_events(&conn)?;

        assert_eq!(replayed, 1);
        let replayed_captures: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events WHERE session_id = 'session-legacy-stable'",
            [],
            |row| row.get(0),
        )?;
        let replayed_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE session_id = 'session-legacy-stable'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(replayed_captures, 1);
        assert_eq!(replayed_events, 1);
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

        spill_capture_event(
            "codex-cli",
            "tool_result-redact",
            &event,
            &summary,
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        let stored = std::fs::read_to_string(crate::db::data_dir().join("capture-spill.jsonl"))?;

        assert!(stored.contains("[REDACTED]"));
        assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!stored.contains("hunter2"));
        assert!(!stored.contains("github_pat_secret"));
        Ok(())
    }

    #[test]
    fn encrypted_capture_spill_hides_payload_and_replays() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-encrypted");
        std::env::set_var("REMEM_CIPHER_KEY", format!("v2:{}", "2".repeat(64)));
        let err = anyhow::anyhow!("database is locked");
        let event = ParsedHookEvent {
            session_id: "session-encrypted-spill".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Write".to_string(),
            tool_input: Some(serde_json::json!({"content": "ordinary source content"})),
            tool_response: Some(serde_json::json!({"ok": true})),
        };
        let summary = EventSummary {
            event_type: "file_write".to_string(),
            summary: "Wrote ordinary source content".to_string(),
            detail: Some("ordinary source content".to_string()),
            files_json: None,
            exit_code: None,
        };

        spill_capture_event(
            "codex-cli",
            "tool_result-encrypted-spill",
            &event,
            &summary,
            SPILL_REASON_DB_OPEN_FAILED,
            &err,
        )?;
        let stored = std::fs::read_to_string(crate::db::data_dir().join("capture-spill.jsonl"))?;
        assert!(stored.contains("remem-spill-v1"));
        assert!(!stored.contains("ordinary source content"));

        let conn = db::open_db()?;
        assert_eq!(replay_spilled_capture_events(&conn)?, 1);
        let replayed_summary: String = conn.query_row(
            "SELECT summary FROM events WHERE session_id = 'session-encrypted-spill'",
            [],
            |row| row.get(0),
        )?;
        assert!(replayed_summary.contains("ordinary source content"));
        Ok(())
    }
}
