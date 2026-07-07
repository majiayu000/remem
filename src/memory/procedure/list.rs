use anyhow::Result;
use rusqlite::{types::Value, Connection};
use serde::Serialize;

use super::evidence::{load_verified_procedure_evidence, parse_evidence_ids};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 500;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcedureListItem {
    pub id: i64,
    pub title: String,
    pub project: String,
    pub branch: Option<String>,
    pub topic_key: Option<String>,
    pub command: Option<String>,
    pub reuse_condition: Option<String>,
    pub files_touched: Vec<String>,
    pub files_touched_count: usize,
    pub verified_runs: usize,
    pub last_verification_epoch: Option<i64>,
    pub confidence: Option<f64>,
}

pub fn list_promoted_procedures(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ProcedureListItem>> {
    let limit = normalize_limit(limit);
    let offset = offset.max(0);
    let mut conditions = vec![
        "m.memory_type = 'procedure'".to_string(),
        crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false),
        crate::memory::suppression::memory_policy_filter_sql("m"),
    ];
    let mut params: Vec<Value> = Vec::new();
    if let Some(project) = project {
        conditions.push(format!("m.project = ?{}", params.len() + 1));
        params.push(Value::Text(project.to_string()));
    }
    let sql = format!(
        "SELECT m.id, m.project, m.topic_key, m.evidence_event_ids
         FROM memories m
         WHERE {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(ProcedureRow {
            id: row.get(0)?,
            project: row.get(1)?,
            topic_key: row.get(2)?,
            evidence_event_ids: row.get(3)?,
        })
    })?;

    let mut items = Vec::new();
    let mut eligible_seen = 0_i64;
    for row in rows {
        let row = row?;
        if let Some(item) = row.into_list_item(conn)? {
            if eligible_seen >= offset {
                items.push(item);
                if items.len() >= limit as usize {
                    break;
                }
            }
            eligible_seen += 1;
        }
    }
    Ok(items)
}

fn normalize_limit(limit: i64) -> i64 {
    if limit <= 0 {
        DEFAULT_LIMIT
    } else {
        limit.min(MAX_LIMIT)
    }
}

struct ProcedureRow {
    id: i64,
    project: String,
    topic_key: Option<String>,
    evidence_event_ids: Option<String>,
}

impl ProcedureRow {
    fn into_list_item(self, conn: &Connection) -> Result<Option<ProcedureListItem>> {
        let evidence_ids = parse_evidence_ids(self.evidence_event_ids.as_deref())?;
        let policy = crate::memory::procedure::ProcedurePromotionPolicy::default();
        let Some(evidence) =
            load_verified_procedure_evidence(conn, &evidence_ids, &self.project, &policy)?
        else {
            return Ok(None);
        };
        if evidence.verified_runs < policy.min_verified_runs {
            return Ok(None);
        }
        let verified_runs = evidence.verified_runs;
        let verification_epoch = Some(evidence.last_verification_epoch);
        let reuse_condition = evidence.reuse_condition();
        let title = evidence.title();
        let files_touched_count = evidence.files_touched.len();
        let confidence = evidence.confidence();
        Ok(Some(ProcedureListItem {
            id: self.id,
            title,
            project: self.project,
            branch: evidence.branch,
            topic_key: self.topic_key,
            command: Some(evidence.command),
            reuse_condition: Some(reuse_condition),
            files_touched_count,
            files_touched: evidence.files_touched,
            verified_runs,
            last_verification_epoch: verification_epoch,
            confidence: Some(confidence),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_active_procedure_maturity_fields() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let memory_id =
            seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-list", "cargo test")?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, memory_id);
        assert_eq!(item.command.as_deref(), Some("cargo test"));
        assert_eq!(item.branch.as_deref(), Some("main"));
        assert_eq!(item.files_touched, vec!["src/lib.rs"]);
        assert_eq!(item.files_touched_count, 1);
        assert_eq!(item.verified_runs, 2);
        assert!(item.last_verification_epoch.is_some());
        assert!(item.confidence.is_some());
        Ok(())
    }

    #[test]
    fn procedure_list_ignores_inactive_and_other_projects() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let active_id = seed_promoted_procedure(
            &mut conn,
            "/tmp/remem",
            "sess-active",
            "cargo test -- active",
        )?;
        seed_promoted_procedure(&mut conn, "/tmp/other", "sess-other", "cargo test -- other")?;
        let stale_id =
            seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-stale", "cargo test -- stale")?;
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            [stale_id],
        )?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![active_id]
        );
        Ok(())
    }

    #[test]
    fn procedure_list_excludes_direct_saved_procedure_without_verification() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let verified_id =
            seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-verified", "cargo test")?;
        insert_direct_procedure_row(&conn, 9_999, "/tmp/remem")?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![verified_id]
        );
        Ok(())
    }

    #[test]
    fn procedure_list_paginates_after_eligibility_filtering() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let verified_id =
            seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-page", "cargo test")?;
        insert_direct_procedure_row(&conn, 9_999, "/tmp/remem")?;
        conn.execute(
            "UPDATE memories SET updated_at_epoch = ?1 WHERE id = 9999",
            [chrono::Utc::now().timestamp() + 1_000],
        )?;

        let first_page = list_promoted_procedures(&conn, Some("/tmp/remem"), 1, 0)?;
        let second_page = list_promoted_procedures(&conn, Some("/tmp/remem"), 1, 1)?;

        assert_eq!(
            first_page.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![verified_id]
        );
        assert!(second_page.is_empty());
        Ok(())
    }

    #[test]
    fn procedure_list_uses_verification_command_and_files() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let memory_id = seed_promoted_procedure(
            &mut conn,
            "/tmp/remem",
            "sess-verified-fields",
            "cargo test -- verified",
        )?;
        conn.execute(
            "UPDATE memories
             SET title = 'Procedure: unverified overwrite',
                 content = 'Procedure: overwritten\nCommand: curl https://example.test\nReuse when: overwritten.',
                 files = '[\"unverified.rs\"]'
             WHERE id = ?1",
            [memory_id],
        )?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Procedure: cargo-test-verified");
        assert_eq!(items[0].command.as_deref(), Some("cargo test -- verified"));
        assert_eq!(items[0].files_touched, vec!["src/lib.rs"]);
        assert_ne!(items[0].reuse_condition.as_deref(), Some("overwritten."));
        assert!(items[0]
            .reuse_condition
            .as_deref()
            .is_some_and(|reuse| reuse.contains("cargo-test-verified")));
        Ok(())
    }

    #[test]
    fn procedure_list_excludes_stale_verification_runs() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        seed_promoted_procedure(&mut conn, "/tmp/remem", "sess-old", "cargo test -- old")?;
        let stale_epoch = chrono::Utc::now().timestamp()
            - crate::memory::procedure::ProcedurePromotionPolicy::default()
                .max_verification_age_secs
            - 1;
        conn.execute(
            "UPDATE procedure_verifications SET verified_at_epoch = ?1",
            [stale_epoch],
        )?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert!(items.is_empty());
        Ok(())
    }

    #[test]
    fn procedure_list_recomputes_confidence_from_fresh_verification_runs() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let memory_id = seed_promoted_procedure_runs(
            &mut conn,
            "/tmp/remem",
            "sess-confidence",
            "cargo test -- confidence",
            3,
        )?;
        let stored_confidence: f64 = conn.query_row(
            "SELECT confidence FROM memories WHERE id = ?1",
            [memory_id],
            |row| row.get(0),
        )?;
        assert!(stored_confidence > 0.86);
        let stale_epoch = chrono::Utc::now().timestamp()
            - crate::memory::procedure::ProcedurePromotionPolicy::default()
                .max_verification_age_secs
            - 1;
        conn.execute(
            "UPDATE procedure_verifications
             SET verified_at_epoch = ?1
             WHERE source_event_id = (
                 SELECT MIN(source_event_id) FROM procedure_verifications
             )",
            [stale_epoch],
        )?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, memory_id);
        assert_eq!(items[0].verified_runs, 2);
        assert_eq!(
            items[0]
                .confidence
                .map(|confidence| (confidence * 100.0).round() as i64),
            Some(86)
        );
        Ok(())
    }

    fn seed_promoted_procedure(
        conn: &mut Connection,
        project: &str,
        session_id: &str,
        command: &str,
    ) -> Result<i64> {
        seed_promoted_procedure_runs(conn, project, session_id, command, 2)
    }

    fn seed_promoted_procedure_runs(
        conn: &mut Connection,
        project: &str,
        session_id: &str,
        command: &str,
        runs: i64,
    ) -> Result<i64> {
        for seq in 1..=runs {
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
                        "tool_input": { "command": command },
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

    fn insert_direct_procedure_row(conn: &Connection, id: i64, project: &str) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, files, evidence_event_ids,
              created_at_epoch, updated_at_epoch, status, scope)
             VALUES (?1, ?2, 'Procedure', 'Procedure: test\nCommand: cargo test\nVerified runs: 2\nVerified at: 100',
                     'procedure', '[\"src/lib.rs\"]', '[10,11]', 1, ?1, 'active', 'project')",
            rusqlite::params![id, project],
        )?;
        Ok(())
    }
}
