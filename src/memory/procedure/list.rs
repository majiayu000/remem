use anyhow::{Context, Result};
use rusqlite::{types::Value, Connection};
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
    let sql = format!(
        "SELECT m.id, m.title, m.project, m.topic_key, m.content,
                m.evidence_event_ids
         FROM memories m
         WHERE {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(ProcedureRow {
            id: row.get(0)?,
            title: row.get(1)?,
            project: row.get(2)?,
            topic_key: row.get(3)?,
            content: row.get(4)?,
            evidence_event_ids: row.get(5)?,
        })
    })?;
    let rows = crate::db::query::collect_rows(rows)?;

    let mut items = Vec::new();
    let mut eligible_seen = 0_i64;
    for row in rows {
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
    title: String,
    project: String,
    topic_key: Option<String>,
    content: String,
    evidence_event_ids: Option<String>,
}

impl ProcedureRow {
    fn into_list_item(self, conn: &Connection) -> Result<Option<ProcedureListItem>> {
        let evidence_ids = parse_evidence_ids(self.evidence_event_ids.as_deref())?;
        let policy = crate::memory::procedure::ProcedurePromotionPolicy::default();
        let Some(verification_summary) =
            verification_summary(conn, &evidence_ids, &self.project, &policy)?
        else {
            return Ok(None);
        };
        if verification_summary.verified_runs < policy.min_verified_runs {
            return Ok(None);
        }
        let verified_runs = verification_summary.verified_runs;
        let verification_epoch = Some(verification_summary.last_verification_epoch);
        Ok(Some(ProcedureListItem {
            id: self.id,
            title: self.title,
            project: self.project,
            branch: verification_summary.branch,
            topic_key: self.topic_key,
            command: Some(verification_summary.command),
            reuse_condition: parse_string_line(&self.content, "Reuse when:"),
            files_touched_count: verification_summary.files_touched.len(),
            files_touched: verification_summary.files_touched,
            verified_runs,
            last_verification_epoch: verification_epoch,
            confidence: Some(super::confidence_for_verified_runs(verified_runs)),
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
    branch: Option<String>,
    command: String,
    files_touched: Vec<String>,
}

fn verification_summary(
    conn: &Connection,
    evidence_ids: &[i64],
    memory_project: &str,
    policy: &crate::memory::procedure::ProcedurePromotionPolicy,
) -> Result<Option<VerificationSummary>> {
    if evidence_ids.is_empty() {
        return Ok(None);
    }
    let earliest = chrono::Utc::now()
        .timestamp()
        .saturating_sub(policy.max_verification_age_secs);
    let placeholders = (1..=evidence_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT p.project_path, v.branch, v.workflow_key, v.command, v.files_touched,
                v.source_event_id, v.verified_at_epoch
         FROM procedure_verifications v
         JOIN projects p ON p.id = v.project_id
         WHERE v.source_event_id IN ({placeholders})
           AND v.verified_at_epoch >= ?
         ORDER BY v.verified_at_epoch ASC, v.source_event_id ASC"
    );
    let mut params = evidence_ids
        .iter()
        .map(|id| Value::Integer(*id))
        .collect::<Vec<_>>();
    params.push(Value::Integer(earliest));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(VerificationRow {
                project: row.get(0)?,
                branch: row.get(1)?,
                workflow_key: row.get(2)?,
                command: row.get(3)?,
                files_touched: row.get(4)?,
                source_event_id: row.get(5)?,
                verified_at_epoch: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    if first.project != memory_project {
        return Ok(None);
    }
    if rows.iter().any(|row| {
        row.project != first.project
            || row.branch != first.branch
            || row.workflow_key != first.workflow_key
            || row.command != first.command
    }) {
        return Ok(None);
    }
    let branch = first.branch.clone();
    let command = first.command.clone();
    let mut source_ids = std::collections::BTreeSet::new();
    let mut files_touched = std::collections::BTreeSet::new();
    let mut last_verification_epoch = first.verified_at_epoch;
    for row in rows {
        source_ids.insert(row.source_event_id);
        last_verification_epoch = last_verification_epoch.max(row.verified_at_epoch);
        for file in parse_files(Some(&row.files_touched))? {
            files_touched.insert(file);
        }
    }
    Ok(Some(VerificationSummary {
        verified_runs: source_ids.len(),
        last_verification_epoch,
        branch,
        command,
        files_touched: files_touched.into_iter().collect(),
    }))
}

struct VerificationRow {
    project: String,
    branch: Option<String>,
    workflow_key: String,
    command: String,
    files_touched: String,
    source_event_id: i64,
    verified_at_epoch: i64,
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
             SET content = 'Procedure: overwritten\nCommand: curl https://example.test\nReuse when: overwritten.',
                 files = '[\"unverified.rs\"]'
             WHERE id = ?1",
            [memory_id],
        )?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].command.as_deref(), Some("cargo test -- verified"));
        assert_eq!(items[0].files_touched, vec!["src/lib.rs"]);
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
