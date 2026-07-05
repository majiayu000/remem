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
                m.files, m.evidence_event_ids, m.confidence, m.updated_at_epoch
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
            updated_at_epoch: row.get(9)?,
        })
    })?;
    let rows = crate::db::query::collect_rows(rows)?;

    rows.into_iter()
        .map(|row| row.into_list_item(conn))
        .collect()
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
    updated_at_epoch: i64,
}

impl ProcedureRow {
    fn into_list_item(self, conn: &Connection) -> Result<ProcedureListItem> {
        let evidence_ids = parse_evidence_ids(self.evidence_event_ids.as_deref())?;
        let verification_epoch = latest_verification_epoch(conn, &evidence_ids)?
            .or_else(|| parse_i64_line(&self.content, "Verified at:"))
            .or(Some(self.updated_at_epoch));
        let verified_runs = evidence_ids
            .len()
            .max(parse_usize_line(&self.content, "Verified runs:").unwrap_or(0));
        let files_touched = parse_files(self.files.as_deref())?;
        Ok(ProcedureListItem {
            id: self.id,
            title: self.title,
            project: self.project,
            branch: self.branch,
            topic_key: self.topic_key,
            command: parse_string_line(&self.content, "Command:"),
            reuse_condition: parse_string_line(&self.content, "Reuse when:"),
            files_touched_count: files_touched.len(),
            files_touched,
            verified_runs,
            last_verification_epoch: verification_epoch,
            confidence: self.confidence,
        })
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

fn latest_verification_epoch(conn: &Connection, evidence_ids: &[i64]) -> Result<Option<i64>> {
    if evidence_ids.is_empty() {
        return Ok(None);
    }
    let placeholders = (1..=evidence_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT MAX(verified_at_epoch)
         FROM procedure_verifications
         WHERE source_event_id IN ({placeholders})"
    );
    let value = conn
        .query_row(
            &sql,
            rusqlite::params_from_iter(evidence_ids.iter()),
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(value)
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

fn parse_i64_line(content: &str, prefix: &str) -> Option<i64> {
    parse_string_line(content, prefix)?.parse().ok()
}

fn parse_usize_line(content: &str, prefix: &str) -> Option<usize> {
    parse_string_line(content, prefix)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_active_procedure_maturity_fields() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let candidate = crate::memory::procedure::build_procedure_candidate(
            &[procedure_trace(10, 1_000), procedure_trace(11, 1_200)],
            1_300,
            &crate::memory::procedure::ProcedurePromotionPolicy::default(),
        )
        .expect("fixture should promote");
        let memory_id = crate::memory::procedure::promote_procedure_memory(&conn, &candidate)?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, memory_id);
        assert_eq!(item.command.as_deref(), Some("cargo test"));
        assert_eq!(item.branch.as_deref(), Some("main"));
        assert_eq!(item.files_touched, vec!["src/lib.rs"]);
        assert_eq!(item.files_touched_count, 1);
        assert_eq!(item.verified_runs, 2);
        assert_eq!(item.last_verification_epoch, Some(1_200));
        assert!(item.confidence.is_some());
        Ok(())
    }

    #[test]
    fn procedure_list_ignores_inactive_and_other_projects() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        insert_procedure_row(&conn, 1, "/tmp/remem", "active")?;
        insert_procedure_row(&conn, 2, "/tmp/other", "active")?;
        insert_procedure_row(&conn, 3, "/tmp/remem", "stale")?;

        let items = list_promoted_procedures(&conn, Some("/tmp/remem"), 10, 0)?;

        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1]
        );
        Ok(())
    }

    fn procedure_trace(
        event_id: i64,
        verified_at_epoch: i64,
    ) -> crate::memory::procedure::ProcedureTrace {
        crate::memory::procedure::ProcedureTrace {
            project: "/tmp/remem".to_string(),
            branch: Some("main".to_string()),
            workflow_key: "release-check".to_string(),
            command: "cargo test".to_string(),
            files_touched: vec!["src/lib.rs".to_string()],
            succeeded: true,
            verified_at_epoch,
            source_event_id: Some(event_id),
        }
    }

    fn insert_procedure_row(conn: &Connection, id: i64, project: &str, status: &str) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, files, evidence_event_ids,
              created_at_epoch, updated_at_epoch, status, scope)
             VALUES (?1, ?2, 'Procedure', 'Procedure: test\nCommand: cargo test\nVerified runs: 2\nVerified at: 100',
                     'procedure', '[\"src/lib.rs\"]', '[10,11]', 1, ?1, ?3, 'project')",
            rusqlite::params![id, project, status],
        )?;
        Ok(())
    }
}
