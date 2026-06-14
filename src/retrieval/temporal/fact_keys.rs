use anyhow::Result;
use chrono::NaiveDate;
use rusqlite::{types::ToSql, Connection, OptionalExtension};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactTimeMode {
    Current,
    AsOf(i64),
}

impl FactTimeMode {
    pub fn from_query(query: &str) -> Self {
        extract_as_of_epoch(query)
            .map(Self::AsOf)
            .unwrap_or(Self::Current)
    }
}

pub fn search_fact_memory_ids(
    conn: &Connection,
    terms: &[&str],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
    include_inactive: bool,
    mode: FactTimeMode,
) -> Result<Vec<i64>> {
    let terms = normalized_fact_terms(terms);
    if terms.is_empty() || limit <= 0 || !sqlite_table_exists(conn, "memory_facts")? {
        return Ok(vec![]);
    }
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let mut conditions = vec![
        "f.source_memory_id IS NOT NULL".to_string(),
        crate::memory::memory_current_filter_sql(
            "m.status",
            "m.expires_at_epoch",
            include_inactive,
        ),
        crate::memory::memory_state_key_current_filter_sql("m"),
    ];
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    let mut idx = 1;
    match mode {
        FactTimeMode::Current => {
            let now = chrono::Utc::now().timestamp();
            conditions.push(crate::memory::facts::current_fact_filter_sql(
                "f",
                has_invalidated_at_epoch,
            ));
            conditions.push(format!(
                "(f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{idx})"
            ));
            conditions.push(format!(
                "(f.valid_to_epoch IS NULL OR f.valid_to_epoch > ?{idx})"
            ));
            params.push(Box::new(now));
            idx += 1;
        }
        FactTimeMode::AsOf(as_of_epoch) => {
            conditions.push(format!(
                "(f.valid_from_epoch IS NULL OR f.valid_from_epoch <= ?{idx})"
            ));
            conditions.push(crate::memory::facts::as_of_validity_filter_sql(
                "f",
                idx,
                has_invalidated_at_epoch,
            ));
            conditions.push(format!("f.learned_at_epoch <= ?{idx}"));
            if has_invalidated_at_epoch {
                conditions.push(format!(
                    "(f.invalidated_at_epoch IS NULL OR f.invalidated_at_epoch > ?{idx})"
                ));
            }
            params.push(Box::new(as_of_epoch));
            idx += 1;
        }
    }
    let mut match_terms = Vec::new();
    for term in terms.iter().take(8) {
        match_terms.push(format!(
            "CASE WHEN f.subject LIKE ?{idx} COLLATE NOCASE \
                    OR f.predicate LIKE ?{idx} COLLATE NOCASE \
                    OR f.object LIKE ?{idx} COLLATE NOCASE \
                  THEN 1 ELSE 0 END"
        ));
        params.push(Box::new(format!("%{term}%")));
        idx += 1;
    }
    if match_terms.is_empty() {
        return Ok(vec![]);
    }
    let required_matches = match_terms.len().min(2);
    let match_score_sql = match_terms.join(" + ");
    conditions.push(format!("({match_score_sql}) >= {required_matches}"));
    if let Some(project) = project {
        conditions.push(crate::retrieval::memory_search::project_or_global_clause(
            "m.project",
            idx,
        ));
        params.push(Box::new(project.to_string()));
        idx += 1;
    }
    if let Some(memory_type) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        params.push(Box::new(memory_type.to_string()));
        idx += 1;
    }
    if let Some(branch) = branch.filter(|branch| !branch.trim().is_empty()) {
        conditions.push(format!("(m.branch = ?{idx} OR m.branch IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }
    params.push(Box::new(limit));
    let sql = format!(
        "SELECT m.id, MAX(COALESCE(f.valid_from_epoch, f.learned_at_epoch)) AS fact_epoch,
                MAX(f.confidence) AS confidence,
                MAX({match_score_sql}) AS match_count
         FROM memory_facts f
         JOIN memories m ON m.id = f.source_memory_id
         WHERE {}
         GROUP BY m.id
         ORDER BY match_count DESC, fact_epoch DESC, confidence DESC, m.updated_at_epoch DESC, m.id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    crate::db::query::collect_rows(rows)
}

pub(crate) fn sqlite_table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn normalized_fact_terms(terms: &[&str]) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for term in terms {
        let term = term
            .trim_matches(|c: char| !(c.is_alphanumeric() || is_cjk(c)))
            .to_lowercase();
        let min_len = if term.chars().any(is_cjk) { 2 } else { 3 };
        if term.chars().count() < min_len
            || term.chars().all(|c| c.is_ascii_digit() || c == '-')
            || is_fact_stop_term(&term)
            || !seen.insert(term.clone())
        {
            continue;
        }
        normalized.push(term);
        if normalized.len() >= 8 {
            break;
        }
    }
    normalized
}

fn is_fact_stop_term(term: &str) -> bool {
    matches!(
        term,
        "after"
            | "before"
            | "current"
            | "during"
            | "from"
            | "latest"
            | "owner"
            | "owned"
            | "owns"
            | "sign"
            | "signer"
            | "signers"
            | "signs"
            | "that"
            | "this"
            | "verified"
            | "verifies"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "with"
    )
}

fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}'
    )
}

fn extract_as_of_epoch(query: &str) -> Option<i64> {
    let lower = query.to_lowercase();
    let markers = ["as of", "as-of", "截至", "截止"];
    for marker in markers {
        let Some(start) = lower.find(marker) else {
            continue;
        };
        let suffix = &lower[start + marker.len()..];
        if let Some(epoch) = first_date_epoch(suffix) {
            return Some(epoch);
        }
    }
    None
}

fn first_date_epoch(text: &str) -> Option<i64> {
    for raw in text.split_whitespace() {
        let token =
            raw.trim_matches(|c: char| !(c.is_ascii_digit() || c == '-' || c == '/' || c == '.'));
        for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"] {
            if let Ok(date) = NaiveDate::parse_from_str(token, fmt) {
                return date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use rusqlite::{params, Connection};

    fn migrated_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_memory(conn: &Connection, id: i64, project: &str, now: i64) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (?1, NULL, ?2, NULL, ?3, ?4, 'decision', NULL, ?5, ?5,
                     'active', NULL, 'project')",
            params![
                id,
                project,
                format!("Memory {id}"),
                "Source memory text has no signer token.",
                now
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_fact(
        conn: &Connection,
        memory_id: i64,
        subject: &str,
        object: &str,
        status: &str,
        valid_from_epoch: Option<i64>,
        valid_to_epoch: Option<i64>,
        learned_at_epoch: i64,
        invalidated_at_epoch: Option<i64>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES ('/repo', ?1, 'verified_by', ?2, ?3, ?4, ?5, ?6, NULL, '[]',
                     0.95, NULL, ?7, ?8, ?5, ?5)",
            params![
                subject,
                object,
                valid_from_epoch,
                valid_to_epoch,
                learned_at_epoch,
                memory_id,
                status,
                invalidated_at_epoch
            ],
        )?;
        Ok(())
    }

    #[test]
    fn parses_as_of_date_markers() {
        let expected = NaiveDate::from_ymd_opt(2026, 5, 4)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(
            FactTimeMode::from_query("owner as of 2026-05-04"),
            FactTimeMode::AsOf(expected)
        );
        assert_eq!(
            FactTimeMode::from_query("owner as-of 2026/05/04"),
            FactTimeMode::AsOf(expected)
        );
        assert_eq!(
            FactTimeMode::from_query("截至 2026.05.04 的 owner"),
            FactTimeMode::AsOf(expected)
        );
    }

    #[test]
    fn current_search_excludes_stale_expired_and_invalidated_facts() -> Result<()> {
        let conn = migrated_conn()?;
        let now = chrono::Utc::now().timestamp();
        for id in 1..=4 {
            insert_memory(&conn, id, "/repo", now - id)?;
        }
        insert_fact(
            &conn,
            1,
            "HarborMint",
            "Toma Reed",
            "active",
            Some(now - 1_000),
            Some(now + 1_000),
            now - 900,
            None,
        )?;
        insert_fact(
            &conn,
            2,
            "HarborMint",
            "Toma Reed",
            "stale",
            Some(now - 1_000),
            Some(now + 1_000),
            now - 800,
            Some(now - 10),
        )?;
        insert_fact(
            &conn,
            3,
            "HarborMint",
            "Toma Reed",
            "active",
            Some(now - 1_000),
            Some(now - 10),
            now - 700,
            None,
        )?;
        insert_fact(
            &conn,
            4,
            "HarborMint",
            "Toma Reed",
            "active",
            Some(now + 10),
            None,
            now - 600,
            None,
        )?;

        let ids = search_fact_memory_ids(
            &conn,
            &["HarborMint", "Toma"],
            Some("/repo"),
            None,
            None,
            10,
            false,
            FactTimeMode::Current,
        )?;

        assert_eq!(ids, vec![1]);
        Ok(())
    }

    #[test]
    fn as_of_search_uses_transaction_time_validity() -> Result<()> {
        let conn = migrated_conn()?;
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 15)
            .and_then(|date| date.and_hms_opt(12, 0, 0))
            .context("valid as-of test date")?
            .and_utc()
            .timestamp();
        for id in 1..=2 {
            insert_memory(&conn, id, "/repo", as_of - id)?;
        }
        insert_fact(
            &conn,
            1,
            "HarborMint",
            "Toma Reed",
            "stale",
            Some(as_of - 10_000),
            Some(as_of + 1_000),
            as_of - 900,
            Some(as_of + 500),
        )?;
        insert_fact(
            &conn,
            2,
            "HarborMint",
            "Toma Reed",
            "active",
            Some(as_of - 10_000),
            None,
            as_of + 100,
            None,
        )?;

        let ids = search_fact_memory_ids(
            &conn,
            &["HarborMint", "Toma"],
            Some("/repo"),
            None,
            None,
            10,
            false,
            FactTimeMode::AsOf(as_of),
        )?;

        assert_eq!(ids, vec![1]);
        Ok(())
    }
}
