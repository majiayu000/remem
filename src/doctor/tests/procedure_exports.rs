use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::{
    db,
    memory::procedure::{
        load_export_eligible_procedure, record_procedure_export, render_procedure_export,
        ProcedureExportFormat, ProcedureExportRecordRequest, ProcedurePromotionPolicy,
    },
};

use super::super::procedure_exports::check_procedure_exports;

#[test]
fn check_procedure_exports_reports_inactive_stale_and_changed_sources() -> Result<()> {
    let mut conn = setup_procedure_export_conn()?;
    let ok_id = seed_doctor_promoted_procedure(&mut conn, "sess-export-ok", "cargo test -- ok")?;
    let inactive_id = seed_doctor_promoted_procedure(
        &mut conn,
        "sess-export-inactive",
        "cargo test -- inactive",
    )?;
    let stale_id =
        seed_doctor_promoted_procedure(&mut conn, "sess-export-stale", "cargo test -- stale")?;
    let changed_updated_at_id = seed_doctor_promoted_procedure(
        &mut conn,
        "sess-export-changed-updated-at",
        "cargo test -- changed-updated-at",
    )?;
    let changed_digest_id = seed_doctor_promoted_procedure(
        &mut conn,
        "sess-export-changed-digest",
        "cargo test -- changed-digest",
    )?;
    let changed_version_id = seed_doctor_promoted_procedure(
        &mut conn,
        "sess-export-changed-version",
        "cargo test -- changed-version",
    )?;

    record_export_snapshot(&conn, ok_id, "ok")?;
    record_export_snapshot(&conn, changed_digest_id, "changed-digest")?;
    record_export_snapshot(&conn, changed_version_id, "changed-version")?;
    record_export_snapshot(&conn, changed_updated_at_id, "changed-updated-at")?;
    record_export_snapshot(&conn, inactive_id, "inactive")?;
    record_export_snapshot(&conn, stale_id, "stale")?;

    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![inactive_id],
    )?;
    let stale_epoch = chrono::Utc::now().timestamp()
        - ProcedurePromotionPolicy::default().max_verification_age_secs
        - 1;
    conn.execute(
        "UPDATE procedure_verifications
         SET verified_at_epoch = ?1
         WHERE command = 'cargo test -- stale'",
        params![stale_epoch],
    )?;
    let changed_updated_at: i64 = conn.query_row(
        "SELECT updated_at_epoch FROM memories WHERE id = ?1",
        params![changed_updated_at_id],
        |row| row.get::<_, i64>(0),
    )? + 60;
    conn.execute(
        "UPDATE memories SET updated_at_epoch = ?1 WHERE id = ?2",
        params![changed_updated_at, changed_updated_at_id],
    )?;
    conn.execute(
        "UPDATE procedure_exports
         SET source_digest = 'content-v1:stale-source-digest'
         WHERE memory_id = ?1",
        params![changed_digest_id],
    )?;
    conn.execute(
        "UPDATE procedure_exports
         SET source_digest_version = source_digest_version + 1
         WHERE memory_id = ?1",
        params![changed_version_id],
    )?;

    let check = check_procedure_exports(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(check.detail.contains("6 export(s) across 1 project(s)"));
    assert!(check.detail.contains("projects: /tmp/remem=6"));
    assert!(check.detail.contains("drifted=5"));
    assert!(check.detail.contains("inactive=1"));
    assert!(check.detail.contains("stale=1"));
    assert!(check.detail.contains("changed=3"));
    assert!(check.detail.contains("source procedure inactive"));
    assert!(check.detail.contains("source verification stale"));
    assert!(check
        .detail
        .contains("source procedure changed after export"));
    Ok(())
}

#[test]
fn check_procedure_exports_reports_clean_registry_as_ok() -> Result<()> {
    let mut conn = setup_procedure_export_conn()?;
    let memory_id =
        seed_doctor_promoted_procedure(&mut conn, "sess-export-clean", "cargo test -- clean")?;
    let other_memory_id = seed_doctor_promoted_procedure_for_project(
        &mut conn,
        "/tmp/remem-other",
        "sess-export-clean-other",
        "cargo test -- clean-other",
    )?;
    record_export_snapshot(&conn, memory_id, "clean")?;
    record_export_snapshot(&conn, other_memory_id, "clean-other")?;

    let check = check_procedure_exports(Some(&conn));

    assert_eq!(check.icon(), "ok");
    assert!(check.detail.contains("2 export(s) across 2 project(s)"));
    assert!(check
        .detail
        .contains("projects: /tmp/remem=1, /tmp/remem-other=1"));
    Ok(())
}

fn setup_procedure_export_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn record_export_snapshot(conn: &Connection, memory_id: i64, label: &str) -> Result<()> {
    let source = load_export_eligible_procedure(conn, memory_id)?;
    let rendered =
        render_procedure_export(&source, ProcedureExportFormat::RunbookMd, 1_700_000_000)?;
    let output_path = format!("/repo/remem-drafts/{label}.runbook.md");
    record_procedure_export(
        conn,
        ProcedureExportRecordRequest {
            source: &source,
            format: ProcedureExportFormat::RunbookMd,
            output_path: Path::new(&output_path),
            content: &rendered,
            cwd: Path::new("/repo"),
            exported_at_epoch: 1_700_000_000,
        },
    )
}

fn seed_doctor_promoted_procedure(
    conn: &mut Connection,
    session_id: &str,
    command: &str,
) -> Result<i64> {
    seed_doctor_promoted_procedure_for_project(conn, "/tmp/remem", session_id, command)
}

fn seed_doctor_promoted_procedure_for_project(
    conn: &mut Connection,
    project: &str,
    session_id: &str,
    command: &str,
) -> Result<i64> {
    for seq in 1..=2 {
        db::record_captured_event(
            conn,
            &db::CaptureEventInput {
                host: "codex-cli",
                session_id,
                project,
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &serde_json::json!({
                    "seq": seq,
                    "event_type": "bash",
                    "exit_code": 0,
                    "tool_input": { "command": command },
                    "files": "[\"src/lib.rs\"]",
                    "git_branch": "main"
                })
                .to_string(),
                task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
            },
        )?;
    }
    let task = db::claim_next_extraction_task(conn, "worker-a", 60)?
        .ok_or_else(|| anyhow::anyhow!("procedure task should be claimed"))?;
    let promoted = crate::memory::procedure::promote_verified_procedures_for_task(
        conn,
        &task,
        &ProcedurePromotionPolicy::default(),
    )?;
    assert_eq!(promoted, 1);
    db::mark_extraction_task_done(conn, task.id, "worker-a", task.high_watermark_event_id)?;
    let memory_id = conn.query_row(
        "SELECT id FROM memories
         WHERE memory_type = 'procedure'
           AND project = ?1
           AND content LIKE '%' || ?2 || '%'
         ORDER BY id DESC
         LIMIT 1",
        params![project, command],
        |row| row.get(0),
    )?;
    Ok(memory_id)
}
