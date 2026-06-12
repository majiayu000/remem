use anyhow::Result;
use rusqlite::Connection;

use crate::db;
use crate::memory::{map_memory_row_pub, Memory};
use crate::retrieval::memory_search::filters::{push_branch_filter, push_project_filter};

#[derive(Debug, Clone)]
pub struct FtsMemoryHit {
    pub memory: Memory,
    pub score: f64,
}

/// FTS5 trigram search on memories.
pub fn search_memories_fts(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Memory>> {
    search_memories_fts_filtered(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        false,
        None,
    )
}

pub fn search_memories_fts_filtered(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    Ok(search_memories_fts_hits_filtered(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_inactive,
        branch,
    )?
    .into_iter()
    .map(|hit| hit.memory)
    .collect())
}

pub fn search_memories_fts_hits_filtered(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_inactive: bool,
    branch: Option<&str>,
) -> Result<Vec<FtsMemoryHit>> {
    let mut conditions = vec!["memories_fts MATCH ?1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(query.to_string())];

    let mut idx = 2;
    conditions.push(crate::memory::memory_current_filter_sql(
        "m.status",
        "m.expires_at_epoch",
        include_inactive,
    ));

    idx = push_project_filter(
        "m.project",
        project,
        idx,
        &mut conditions,
        &mut param_values,
    );
    idx = push_branch_filter("m.branch", branch, idx, &mut conditions, &mut param_values);
    if let Some(memory_type) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(memory_type.to_string()));
        idx += 1;
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "WITH ranked AS (
             SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content,
                    m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch,
                    m.status, m.branch, m.scope,
                    (bm25(memories_fts, 10.0, 1.0, 3.0) * CASE WHEN m.memory_type IN ('decision','bugfix') THEN 1.5 ELSE 1.0 END) AS rank_score
             FROM memories m
             JOIN memories_fts ON memories_fts.rowid = m.id
             WHERE {}
         )
         SELECT id, session_id, project, topic_key, title, content,
                memory_type, files, created_at_epoch, updated_at_epoch,
                status, branch, scope, rank_score
         FROM ranked
         ORDER BY rank_score
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(FtsMemoryHit {
            memory: map_memory_row_pub(row)?,
            score: row.get(13)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate::run_migrations(&conn).unwrap();
        conn
    }

    fn insert_memory(conn: &Connection, id: i64, body: &str, status: &str) {
        conn.execute(
            "INSERT INTO memories(id, project, title, content, memory_type, created_at_epoch,
                updated_at_epoch, status)
             VALUES (?1, 'proj', 'title', ?2, 'decision', 100, 100, ?3)",
            rusqlite::params![id, body, status],
        )
        .unwrap();
    }

    /// Reproduction for #236: before v019 a stale row never entered memories_fts,
    /// so the JOIN dropped it and include_inactive bm25 search returned empty.
    #[test]
    fn include_inactive_finds_stale_rows() {
        let conn = setup_conn();
        insert_memory(&conn, 1, "deprecated zookeeper approach", "stale");

        // active-only search must hide the stale row
        let active_only = search_memories_fts_filtered(
            &conn,
            "zookeeper",
            Some("proj"),
            None,
            10,
            0,
            false,
            None,
        )
        .unwrap();
        assert!(
            active_only.is_empty(),
            "active-only search must hide stale rows: {active_only:?}"
        );

        // include_inactive must surface the stale row via the bm25 path
        let with_inactive =
            search_memories_fts_filtered(&conn, "zookeeper", Some("proj"), None, 10, 0, true, None)
                .unwrap();
        assert_eq!(
            with_inactive.len(),
            1,
            "include_inactive must retrieve stale"
        );
        assert_eq!(with_inactive[0].status, "stale");
    }

    /// Active rows must remain retrievable on the default (active-only) path.
    #[test]
    fn active_path_still_finds_active_rows() {
        let conn = setup_conn();
        insert_memory(&conn, 1, "current kafka pipeline", "active");

        let hits =
            search_memories_fts_filtered(&conn, "kafka", Some("proj"), None, 10, 0, false, None)
                .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].status, "active");
    }
}
