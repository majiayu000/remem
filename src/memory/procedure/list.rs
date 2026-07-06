use anyhow::{Context, Result};
use rusqlite::{types::Value, Connection, OptionalExtension};
use serde::Serialize;

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
    let limit_idx = params.len() + 1;
    params.push(Value::Integer(limit));
    let offset_idx = params.len() + 1;
    params.push(Value::Integer(offset));

    let sql = format!(
        "SELECT m.id, m.title, m.project, m.branch, m.topic_key, m.content,
                m.files, m.evidence_event_ids, m.confidence
         FROM memories m
         WHERE {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(ProcedureRow {
            id: row.get(0)?,
            title: row.get(1)?,
            project: row.get(2)?,
            branch: row.get(3)?,
            topic_key: row.get(4)?,
            content: row.get(5)?,
            files: row.get(6)?,
            evidence_event_ids: row.get(7)?,
            confidence: row.get(8)?,
        })
    })?;
    let rows = crate::db::query::collect_rows(rows)?;

    let mut items = Vec::new();
    for row in rows {
        if let Some(item) = row.into_list_item(conn)? {
            items.push(item);
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
    title: String,
    project: String,
    branch: Option<String>,
    topic_key: Option<String>,
    content: String,
    files: Option<String>,
    evidence_event_ids: Option<String>,
    confidence: Option<f64>,
}

impl ProcedureRow {
    fn into_list_item(self, conn: &Connection) -> Result<Option<ProcedureListItem>> {
        let evidence_ids = parse_evidence_ids(self.evidence_event_ids.as_deref())?;
        let Some(verification_summary) = verification_summary(conn, &evidence_ids)? else {
            return Ok(None);
        };
        if verification_summary.verified_runs
            < crate::memory::procedure::ProcedurePromotionPolicy::default().min_verified_runs
        {
            return Ok(None);
        }
        let verification_epoch = Some(verification_summary.last_verification_epoch);
        let files_touched = parse_files(self.files.as_deref())?;
        Ok(Some(ProcedureListItem {
            id: self.id,
            title: self.title,
            project: self.project,
            branch: self.branch,
            topic_key: self.topic_key,
            command: parse_string_line(&self.content, "Command:"),
            reuse_condition: parse_string_line(&self.content, "Reuse when:"),
            files_touched_count: files_touched.len(),
            files_touched,
            verified_runs: verification_summary.verified_runs,
            last_verification_epoch: verification_epoch,
            confidence: self.confidence,
        }))
    }
}

fn parse_evidence_ids(raw: Option<&str>) -> Result<Vec<i64>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).with_context(|| "invalid procedure evidence_event_ids JSON")
}

struct VerificationSummary {
    verified_runs: usize,
    last_verification_epoch: i64,
}

fn verification_summary(
    conn: &Connection,
    evidence_ids: &[i64],
) -> Result<Option<VerificationSummary>> {
    if evidence_ids.is_empty() {
        return Ok(None);
    }
    let placeholders = (1..=evidence_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT COUNT(DISTINCT source_event_id), MAX(verified_at_epoch)
         FROM procedure_verifications
         WHERE source_event_id IN ({placeholders})"
    );
    let (verified_runs, last_verification_epoch): (i64, Option<i64>) = conn
        .query_row(
            &sql,
            rusqlite::params_from_iter(evidence_ids.iter()),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or((0, None));
    let Some(last_verification_epoch) = last_verification_epoch else {
        return Ok(None);
    };
    Ok(Some(VerificationSummary {
        verified_runs: verified_runs.max(0) as usize,
        last_verification_epoch,
    }))
}

fn parse_files(raw: Option<&str>) -> Result<Vec<String>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).with_context(|| "invalid procedure files JSON")
}

fn parse_string_line(content: &str, prefix: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.trim()
            .strip_prefix(prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
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

    fn seed_promoted_procedure(
        conn: &mut Connection,
        project: &str,
        session_id: &str,
        command: &str,
    ) -> Result<i64> {
        for seq in [1, 2] {
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
