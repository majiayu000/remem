use anyhow::Result;
use rusqlite::Connection;

use super::{promote_verified_procedures_for_task, ProcedurePromotionPolicy};

fn record_bash_event(conn: &Connection, session_id: &str, seq: i64) -> Result<i64> {
    let outcome = crate::db::record_captured_event(
        conn,
        &crate::db::CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: &serde_json::json!({
                "seq": seq,
                "event_type": "bash",
                "exit_code": 0,
                "tool_input": { "command": "cargo test" },
                "files": "[\"src/lib.rs\"]",
                "git_branch": "main"
            })
            .to_string(),
            task_kind: None,
        },
    )?;
    Ok(outcome.event_row_id)
}

#[test]
fn production_task_does_not_rescan_unstored_prior_bash_events() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    let session_id = "sess-procedure-incremental";

    record_bash_event(&conn, session_id, 1)?;
    let old_high_watermark = record_bash_event(&conn, session_id, 2)?;
    let current_event_id = record_bash_event(&conn, session_id, 3)?;

    let (host_id, workspace_id, project_id, session_row_id): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT host_id, workspace_id, project_id, session_row_id
             FROM captured_events
             WHERE id = ?1",
            [current_event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    let task = crate::db::ExtractionTask {
        id: 1,
        task_kind: crate::db::ExtractionTaskKind::ObservationExtract,
        host_id,
        workspace_id,
        project_id,
        session_row_id: Some(session_row_id),
        host: "codex-cli".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: Some(session_id.to_string()),
        ai_profile: None,
        priority: crate::db::ExtractionTaskKind::ObservationExtract.priority(),
        cursor_event_id: Some(old_high_watermark),
        high_watermark_event_id: Some(current_event_id),
        attempts: 0,
    };

    let promoted =
        promote_verified_procedures_for_task(&conn, &task, &ProcedurePromotionPolicy::default())?;

    assert_eq!(promoted, 0);
    let trace_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM procedure_verifications", [], |row| {
            row.get(0)
        })?;
    assert_eq!(trace_count, 1);
    let procedure_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE memory_type = 'procedure'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(procedure_count, 0);
    Ok(())
}
