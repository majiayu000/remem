use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::evidence::{load_verified_procedure_evidence, parse_evidence_ids};

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) struct ProcedureExportSource {
    pub(crate) id: i64,
    pub(crate) project: String,
    pub(crate) branch: Option<String>,
    pub(crate) topic_key: Option<String>,
    pub(crate) title: String,
    pub(crate) stored_title: String,
    pub(crate) stored_content: String,
    pub(crate) workflow_key: String,
    pub(crate) command: String,
    pub(crate) reuse_condition: String,
    pub(crate) files_touched: Vec<String>,
    pub(crate) evidence_event_ids: Vec<i64>,
    pub(crate) verified_runs: usize,
    pub(crate) last_verification_epoch: i64,
    pub(crate) confidence: f64,
    pub(crate) source_updated_at_epoch: i64,
}

#[allow(dead_code)]
pub(crate) fn load_export_eligible_procedure(
    conn: &Connection,
    memory_id: i64,
) -> Result<ProcedureExportSource> {
    if memory_id <= 0 {
        bail!("procedure memory id must be positive");
    }
    let row = load_procedure_row(conn, memory_id)?
        .with_context(|| format!("procedure memory {memory_id} was not found"))?;
    if row.memory_type != "procedure" {
        bail!(
            "memory {memory_id} is not export eligible: expected memory_type 'procedure', found '{}'",
            row.memory_type
        );
    }
    if row.status != "active" {
        bail!(
            "procedure memory {memory_id} is not export eligible: source status is '{}'",
            row.status
        );
    }
    if row
        .expires_at_epoch
        .is_some_and(|expires_at| expires_at <= chrono::Utc::now().timestamp())
    {
        bail!("procedure memory {memory_id} is not export eligible: source is expired");
    }
    ensure_policy_visible(conn, memory_id)?;
    ensure_current_state(conn, memory_id)?;

    let evidence_event_ids = parse_evidence_ids(row.evidence_event_ids.as_deref())
        .with_context(|| format!("procedure memory {memory_id} has invalid evidence_event_ids"))?;
    let policy = super::ProcedurePromotionPolicy::default();
    let Some(evidence) =
        load_verified_procedure_evidence(conn, &evidence_event_ids, &row.project, &policy)?
    else {
        bail!(
            "procedure memory {memory_id} is not export eligible: fresh verification evidence is missing or inconsistent"
        );
    };
    if evidence.verified_runs < policy.min_verified_runs {
        bail!(
            "procedure memory {memory_id} is not export eligible: only {} fresh verified run(s), need {}",
            evidence.verified_runs,
            policy.min_verified_runs
        );
    }

    let title = evidence.title();
    let reuse_condition = evidence.reuse_condition();
    let confidence = evidence.confidence();
    Ok(ProcedureExportSource {
        id: row.id,
        project: row.project,
        branch: evidence.branch,
        topic_key: row.topic_key,
        title,
        stored_title: row.title,
        stored_content: row.content,
        workflow_key: evidence.workflow_key,
        command: evidence.command,
        reuse_condition,
        files_touched: evidence.files_touched,
        evidence_event_ids: evidence.source_event_ids,
        verified_runs: evidence.verified_runs,
        last_verification_epoch: evidence.last_verification_epoch,
        confidence,
        source_updated_at_epoch: row.updated_at_epoch,
    })
}

struct ProcedureMemoryRow {
    id: i64,
    project: String,
    topic_key: Option<String>,
    title: String,
    content: String,
    memory_type: String,
    status: String,
    expires_at_epoch: Option<i64>,
    updated_at_epoch: i64,
    evidence_event_ids: Option<String>,
}

fn load_procedure_row(conn: &Connection, memory_id: i64) -> Result<Option<ProcedureMemoryRow>> {
    conn.query_row(
        "SELECT id, project, topic_key, title, content, memory_type, status,
                expires_at_epoch, updated_at_epoch, evidence_event_ids
         FROM memories
         WHERE id = ?1",
        params![memory_id],
        |row| {
            Ok(ProcedureMemoryRow {
                id: row.get(0)?,
                project: row.get(1)?,
                topic_key: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                memory_type: row.get(5)?,
                status: row.get(6)?,
                expires_at_epoch: row.get(7)?,
                updated_at_epoch: row.get(8)?,
                evidence_event_ids: row.get(9)?,
            })
        },
    )
    .optional()
    .context("load procedure memory row")
}

fn ensure_policy_visible(conn: &Connection, memory_id: i64) -> Result<()> {
    let sql = format!(
        "SELECT COUNT(*)
         FROM memories m
         WHERE m.id = ?1
           AND {}",
        crate::memory::suppression::memory_policy_filter_sql("m")
    );
    let count: i64 = conn.query_row(&sql, params![memory_id], |row| row.get(0))?;
    if count == 0 {
        bail!("procedure memory {memory_id} is not export eligible: source is policy-suppressed");
    }
    Ok(())
}

fn ensure_current_state(conn: &Connection, memory_id: i64) -> Result<()> {
    let state_key_sql = format!(
        "SELECT COUNT(*)
         FROM memories m
         WHERE m.id = ?1
           AND {}",
        crate::memory::memory_state_key_current_filter_sql("m")
    );
    let current_count: i64 =
        conn.query_row(&state_key_sql, params![memory_id], |row| row.get(0))?;
    if current_count == 0 {
        bail!("procedure memory {memory_id} is not export eligible: source is not current");
    }

    let supersede_sql = format!(
        "SELECT COUNT(*)
         FROM memories m
         WHERE m.id = ?1
           AND {}",
        crate::memory::memory_not_superseded_filter_sql("m")
    );
    let not_superseded_count: i64 =
        conn.query_row(&supersede_sql, params![memory_id], |row| row.get(0))?;
    if not_superseded_count == 0 {
        bail!("procedure memory {memory_id} is not export eligible: source is superseded");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn export_eligibility_loads_active_verified_procedure() -> Result<()> {
        let mut conn = setup_conn()?;
        let memory_id = seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-export-ok")?;

        let source = load_export_eligible_procedure(&conn, memory_id)?;

        assert_eq!(source.id, memory_id);
        assert_eq!(source.project, "/tmp/remem");
        assert_eq!(source.branch.as_deref(), Some("main"));
        assert_eq!(source.title, "Procedure: cargo-test");
        assert_eq!(source.workflow_key, "cargo-test");
        assert_eq!(source.command, "cargo test");
        assert_eq!(source.files_touched, vec!["src/lib.rs"]);
        assert_eq!(source.verified_runs, 2);
        assert_eq!(source.evidence_event_ids.len(), 2);
        assert!(source.reuse_condition.contains("cargo-test"));
        assert!(source.confidence >= 0.86);
        assert!(source.source_updated_at_epoch > 0);
        Ok(())
    }

    #[test]
    fn export_eligibility_rejects_non_procedure_memory() -> Result<()> {
        let conn = setup_conn()?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, scope)
             VALUES (91, '/tmp/remem', 'Decision', 'Use cargo test.', 'decision', 1, 1, 'active', 'project')",
            [],
        )?;

        let err = load_export_eligible_procedure(&conn, 91).expect_err("decision must reject");

        assert!(err.to_string().contains("expected memory_type 'procedure'"));
        Ok(())
    }

    #[test]
    fn export_eligibility_rejects_inactive_procedure() -> Result<()> {
        let mut conn = setup_conn()?;
        let memory_id = seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-export-stale")?;
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            params![memory_id],
        )?;

        let err = load_export_eligible_procedure(&conn, memory_id)
            .expect_err("inactive procedure must reject");

        assert!(err.to_string().contains("source status is 'stale'"));
        Ok(())
    }

    #[test]
    fn export_eligibility_rejects_stale_verification_evidence() -> Result<()> {
        let mut conn = setup_conn()?;
        let memory_id = seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-export-old")?;
        let stale_epoch = chrono::Utc::now().timestamp()
            - super::super::ProcedurePromotionPolicy::default().max_verification_age_secs
            - 1;
        conn.execute(
            "UPDATE procedure_verifications SET verified_at_epoch = ?1",
            params![stale_epoch],
        )?;

        let err = load_export_eligible_procedure(&conn, memory_id)
            .expect_err("stale evidence must reject");

        assert!(err.to_string().contains("fresh verification evidence"));
        Ok(())
    }

    #[test]
    fn export_eligibility_rejects_policy_suppressed_procedure() -> Result<()> {
        let mut conn = setup_conn()?;
        let memory_id = seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-export-suppressed")?;
        crate::memory::suppression::create_suppression(
            &conn,
            &crate::memory::suppression::SuppressRequest {
                target: crate::memory::suppression::SuppressionTarget {
                    kind: "memory".to_string(),
                    id: Some(memory_id),
                    value: None,
                },
                reason: Some("review withheld"),
                actor: Some("test"),
            },
        )?;

        let err = load_export_eligible_procedure(&conn, memory_id)
            .expect_err("suppressed procedure must reject");

        assert!(err.to_string().contains("policy-suppressed"));
        Ok(())
    }

    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn seed_promoted_procedure(
        conn: &mut Connection,
        project: &str,
        session_id: &str,
    ) -> Result<i64> {
        for seq in 1..=2 {
            crate::db::record_captured_event(
                conn,
                &crate::db::CaptureEventInput {
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
                        "tool_input": { "command": "cargo test" },
                        "files": "[\"src/lib.rs\"]",
                        "git_branch": "main"
                    })
                    .to_string(),
                    task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
                },
            )?;
        }
        let task = crate::db::claim_next_extraction_task(conn, "worker-a", 60)?
            .ok_or_else(|| anyhow::anyhow!("procedure task should be claimed"))?;
        let promoted = crate::memory::procedure::promote_verified_procedures_for_task(
            conn,
            &task,
            &crate::memory::procedure::ProcedurePromotionPolicy::default(),
        )?;
        assert_eq!(promoted, 1);
        let memory_id = conn.query_row(
            "SELECT id FROM memories WHERE memory_type = 'procedure' AND project = ?1
             ORDER BY id DESC LIMIT 1",
            [project],
            |row| row.get(0),
        )?;
        crate::db::mark_extraction_task_done(
            conn,
            task.id,
            "worker-a",
            task.high_watermark_event_id,
        )?;
        Ok(memory_id)
    }
}
