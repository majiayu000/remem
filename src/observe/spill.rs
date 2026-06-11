use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureSpillStats {
    pub pending_files: i64,
    pub pending_bytes: i64,
    pub latest_epoch: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureSpillWrite {
    pub path: PathBuf,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CaptureSpillRecord {
    pub schema_version: u8,
    pub created_at_epoch: i64,
    pub event_id: String,
    pub host: String,
    pub adapter: String,
    pub session_id: String,
    pub project: String,
    pub cwd: Option<String>,
    pub event_type: String,
    pub role: Option<String>,
    pub tool_name: Option<String>,
    pub content: String,
    pub summary_event_type: String,
    pub summary: String,
    pub summary_detail: Option<String>,
    pub summary_files_json: Option<String>,
    pub summary_exit_code: Option<i32>,
    pub db_error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplayCaptureSpillStats {
    pub replayed: usize,
    pub failed: usize,
}

const SPILL_SCHEMA_VERSION: u8 = 1;

pub(super) fn write_capture_spill(
    event_id: &str,
    host: &str,
    adapter: &str,
    event: &crate::adapter::ParsedHookEvent,
    summary: &crate::adapter::EventSummary,
    content: &str,
    db_error: &str,
) -> Result<CaptureSpillWrite> {
    let now = chrono::Utc::now().timestamp();
    let redacted_content = crate::db::capture::redact_capture_content(content);
    let redacted_summary = crate::db::capture::redact_capture_content(&summary.summary);
    let redacted_detail = summary
        .detail
        .as_ref()
        .map(|detail| crate::db::capture::redact_capture_content(detail));
    let redacted_error = crate::db::capture::redact_capture_content(db_error);
    let record = CaptureSpillRecord {
        schema_version: SPILL_SCHEMA_VERSION,
        created_at_epoch: now,
        event_id: event_id.to_string(),
        host: host.to_string(),
        adapter: adapter.to_string(),
        session_id: event.session_id.clone(),
        project: event.project.clone(),
        cwd: event.cwd.clone(),
        event_type: "tool_result".to_string(),
        role: None,
        tool_name: Some(event.tool_name.clone()),
        content: redacted_content,
        summary_event_type: summary.event_type.clone(),
        summary: redacted_summary,
        summary_detail: redacted_detail,
        summary_files_json: summary.files_json.clone(),
        summary_exit_code: summary.exit_code,
        db_error: crate::db::truncate_str(&redacted_error, 512).to_string(),
    };
    let dir = capture_spill_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create capture spill dir {}", dir.display()))?;
    let path = dir.join(format!(
        "observe-{}-{}.json",
        now,
        sanitize_file_component(event_id)
    ));
    let bytes = serde_json::to_vec_pretty(&record)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(&bytes)
        })
        .with_context(|| format!("write capture spill {}", path.display()))?;
    Ok(CaptureSpillWrite {
        path,
        event_id: event_id.to_string(),
    })
}

pub(crate) fn capture_spill_stats() -> Result<CaptureSpillStats> {
    let dir = capture_spill_dir();
    if !dir.exists() {
        return Ok(CaptureSpillStats {
            pending_files: 0,
            pending_bytes: 0,
            latest_epoch: None,
        });
    }

    let mut pending_files = 0_i64;
    let mut pending_bytes = 0_i64;
    let mut latest_epoch = None;
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("read capture spill dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        pending_files += 1;
        pending_bytes += i64::try_from(meta.len()).unwrap_or(i64::MAX);
        if let Some(epoch) = spill_epoch_from_path(&path) {
            latest_epoch = Some(latest_epoch.map_or(epoch, |current: i64| current.max(epoch)));
        }
    }

    Ok(CaptureSpillStats {
        pending_files,
        pending_bytes,
        latest_epoch,
    })
}

pub(super) fn replay_capture_spills(conn: &Connection) -> Result<ReplayCaptureSpillStats> {
    let dir = capture_spill_dir();
    if !dir.exists() {
        return Ok(ReplayCaptureSpillStats {
            replayed: 0,
            failed: 0,
        });
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("read capture spill dir {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && entry.metadata()?.is_file()
        {
            paths.push(path);
        }
    }
    paths.sort();

    let mut stats = ReplayCaptureSpillStats {
        replayed: 0,
        failed: 0,
    };
    for path in paths {
        match replay_one_capture_spill(conn, &path) {
            Ok(()) => stats.replayed += 1,
            Err(error) => {
                stats.failed += 1;
                let detail = format!("{}: {}", path.display(), error);
                let _ = crate::db::record_capture_audit_event(
                    conn,
                    &crate::db::CaptureAuditInput {
                        host: None,
                        adapter: None,
                        session_id: None,
                        project: None,
                        cwd: None,
                        tool_name: None,
                        reason: "spill_replay_failed",
                        detail: Some(&detail),
                        payload: None,
                    },
                );
                crate::log::warn("observe", &format!("capture spill replay failed: {detail}"));
            }
        }
    }
    Ok(stats)
}

fn replay_one_capture_spill(conn: &Connection, path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read capture spill {}", path.display()))?;
    let record: CaptureSpillRecord = serde_json::from_str(&text)
        .with_context(|| format!("parse capture spill {}", path.display()))?;
    anyhow::ensure!(
        record.schema_version == SPILL_SCHEMA_VERSION,
        "unsupported spill schema version {}",
        record.schema_version
    );
    crate::db::record_captured_event_with_id(
        conn,
        &crate::db::CaptureEventInput {
            host: &record.host,
            session_id: &record.session_id,
            project: &record.project,
            cwd: record.cwd.as_deref(),
            event_type: &record.event_type,
            role: record.role.as_deref(),
            tool_name: record.tool_name.as_deref(),
            content: &record.content,
            task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
        },
        Some(&record.event_id),
    )?;
    insert_memory_event_once(conn, &record)?;
    crate::db::record_capture_audit_event(
        conn,
        &crate::db::CaptureAuditInput {
            host: Some(&record.host),
            adapter: Some(&record.adapter),
            session_id: Some(&record.session_id),
            project: Some(&record.project),
            cwd: record.cwd.as_deref(),
            tool_name: record.tool_name.as_deref(),
            reason: "spill_replayed",
            detail: Some(&record.event_id),
            payload: Some(&record.content),
        },
    )?;
    std::fs::remove_file(path)
        .with_context(|| format!("remove replayed capture spill {}", path.display()))?;
    Ok(())
}

fn insert_memory_event_once(conn: &Connection, record: &CaptureSpillRecord) -> Result<()> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM events
             WHERE session_id = ?1
               AND project = ?2
               AND event_type = ?3
               AND summary = ?4
               AND COALESCE(detail, '') = COALESCE(?5, '')
               AND COALESCE(files, '') = COALESCE(?6, '')
               AND COALESCE(exit_code, -2147483648) = COALESCE(?7, -2147483648)
             LIMIT 1",
            params![
                &record.session_id,
                &record.project,
                &record.summary_event_type,
                &record.summary,
                record.summary_detail.as_deref(),
                record.summary_files_json.as_deref(),
                record.summary_exit_code
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        crate::memory::insert_event(
            conn,
            &record.session_id,
            &record.project,
            &record.summary_event_type,
            &record.summary,
            record.summary_detail.as_deref(),
            record.summary_files_json.as_deref(),
            record.summary_exit_code,
        )?;
    }
    Ok(())
}

fn capture_spill_dir() -> PathBuf {
    crate::db::data_dir().join("capture-spill")
}

fn spill_epoch_from_path(path: &Path) -> Option<i64> {
    path.file_name()
        .and_then(|value| value.to_str())
        .and_then(|name| name.strip_prefix("observe-"))
        .and_then(|name| name.split('-').next())
        .and_then(|epoch| epoch.parse::<i64>().ok())
}

fn sanitize_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{EventSummary, ParsedHookEvent};
    use crate::db::{self, test_support::ScopedTestDataDir};

    #[test]
    fn capture_spill_round_trips_and_replays_once() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-replay");
        let event = ParsedHookEvent {
            session_id: "sess-spill".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Edit".to_string(),
            tool_input: Some(serde_json::json!({ "file_path": "src/lib.rs" })),
            tool_response: Some(serde_json::json!({ "content": "edited" })),
        };
        let summary = EventSummary {
            event_type: "file_edit".to_string(),
            summary: "Edit src/lib.rs".to_string(),
            detail: None,
            files_json: Some(r#"["src/lib.rs"]"#.to_string()),
            exit_code: None,
        };
        let content = serde_json::json!({
            "summary": summary.summary,
            "tool_name": event.tool_name,
            "tool_input": event.tool_input,
        })
        .to_string();

        let event_id = db::unique_capture_event_id("tool_result", &content);
        let written = write_capture_spill(
            &event_id,
            "claude-code",
            "claude-code",
            &event,
            &summary,
            &content,
            "db",
        )?;
        assert!(written.path.exists());
        assert_eq!(capture_spill_stats()?.pending_files, 1);

        let conn = db::open_db()?;
        let stats = replay_capture_spills(&conn)?;
        assert_eq!(stats.replayed, 1);
        assert_eq!(stats.failed, 0);
        assert_eq!(capture_spill_stats()?.pending_files, 0);
        let captured: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        let events: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        let replayed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM capture_audit_events WHERE reason = 'spill_replayed'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(captured, 1);
        assert_eq!(events, 1);
        assert_eq!(replayed, 1);
        Ok(())
    }

    #[test]
    fn repeated_identical_spills_get_distinct_files() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-distinct");
        let event = ParsedHookEvent {
            session_id: "sess-spill-distinct".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Edit".to_string(),
            tool_input: Some(serde_json::json!({ "file_path": "src/lib.rs" })),
            tool_response: None,
        };
        let summary = EventSummary {
            event_type: "file_edit".to_string(),
            summary: "Edit src/lib.rs".to_string(),
            detail: None,
            files_json: Some(r#"["src/lib.rs"]"#.to_string()),
            exit_code: None,
        };
        let content = serde_json::json!({
            "summary": summary.summary,
            "tool_name": event.tool_name,
            "tool_input": event.tool_input,
        })
        .to_string();

        let first_id = db::unique_capture_event_id("tool_result", &content);
        let second_id = db::unique_capture_event_id("tool_result", &content);
        let first = write_capture_spill(
            &first_id,
            "claude-code",
            "claude-code",
            &event,
            &summary,
            &content,
            "db",
        )?;
        let second = write_capture_spill(
            &second_id,
            "claude-code",
            "claude-code",
            &event,
            &summary,
            &content,
            "db",
        )?;

        assert_ne!(first.event_id, second.event_id);
        assert_ne!(first.path, second.path);
        assert_eq!(capture_spill_stats()?.pending_files, 2);
        Ok(())
    }

    #[test]
    fn capture_spill_redacts_free_form_fields() -> Result<()> {
        let _test_dir = ScopedTestDataDir::new("capture-spill-redacts");
        let event = ParsedHookEvent {
            session_id: "sess-spill-redact".to_string(),
            cwd: Some("/tmp/remem".to_string()),
            project: "/tmp/remem".to_string(),
            tool_name: "Bash".to_string(),
            tool_input: Some(serde_json::json!({
                "command": "curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456'"
            })),
            tool_response: Some(serde_json::json!({
                "stderr": "password=hunter2"
            })),
        };
        let summary = EventSummary {
            event_type: "bash".to_string(),
            summary: "Run `curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456'`"
                .to_string(),
            detail: Some("password=hunter2".to_string()),
            files_json: None,
            exit_code: Some(1),
        };
        let content = serde_json::json!({
            "summary": summary.summary,
            "detail": summary.detail,
            "tool_name": event.tool_name,
            "tool_input": event.tool_input,
            "tool_response": event.tool_response,
        })
        .to_string();
        let event_id = db::unique_capture_event_id("tool_result", &content);

        let written = write_capture_spill(
            &event_id,
            "codex-cli",
            "codex-cli",
            &event,
            &summary,
            &content,
            "database token=github_pat_secret",
        )?;
        let stored = std::fs::read_to_string(written.path)?;

        assert!(stored.contains("[REDACTED]"));
        assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!stored.contains("hunter2"));
        assert!(!stored.contains("github_pat_secret"));
        Ok(())
    }
}
