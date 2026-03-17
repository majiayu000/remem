use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db;

// --- Data Models ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub session_id: Option<String>,
    pub project: String,
    pub topic_key: Option<String>,
    pub title: String,
    pub text: String,
    pub memory_type: String,
    pub files: Option<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub event_type: String,
    pub summary: String,
    pub detail: Option<String>,
    pub files: Option<String>,
    pub exit_code: Option<i32>,
    pub created_at_epoch: i64,
}

pub const MEMORY_TYPES: &[&str] = &[
    "decision",
    "discovery",
    "bugfix",
    "architecture",
    "preference",
    "session_activity",
];

// --- Memory CRUD ---

/// Insert or update a memory. If topic_key is provided and a matching
/// (project, topic_key) row exists, update it instead of inserting.
pub fn insert_memory(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();

    // UPSERT: if topic_key is set, try to find existing
    if let Some(tk) = topic_key {
        if !tk.is_empty() {
            let existing_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM memories WHERE project = ?1 AND topic_key = ?2 LIMIT 1",
                    params![project, tk],
                    |row| row.get(0),
                )
                .ok();

            if let Some(id) = existing_id {
                conn.execute(
                    "UPDATE memories SET session_id = ?1, title = ?2, content = ?3, \
                     memory_type = ?4, files = ?5, updated_at_epoch = ?6 \
                     WHERE id = ?7",
                    params![session_id, title, content, memory_type, files, now, id],
                )?;
                return Ok(id);
            }
        }
    }

    conn.execute(
        "INSERT INTO memories \
         (session_id, project, topic_key, title, content, memory_type, files, \
          created_at_epoch, updated_at_epoch, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'active')",
        params![
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_recent_memories(conn: &Connection, project: &str, limit: i64) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, topic_key, title, content, memory_type, files, \
         created_at_epoch, updated_at_epoch, status \
         FROM memories \
         WHERE project = ?1 AND status = 'active' \
         ORDER BY updated_at_epoch DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn get_memories_by_type(
    conn: &Connection,
    project: &str,
    memory_type: &str,
    limit: i64,
) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, topic_key, title, content, memory_type, files, \
         created_at_epoch, updated_at_epoch, status \
         FROM memories \
         WHERE project = ?1 AND memory_type = ?2 AND status = 'active' \
         ORDER BY updated_at_epoch DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project, memory_type, limit], map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn get_memories_by_ids(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
) -> Result<Vec<Memory>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let mut conditions = vec![format!("id IN ({})", placeholders.join(", "))];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();

    if let Some(p) = project {
        conditions.push(format!("project = ?{}", ids.len() + 1));
        param_values.push(Box::new(p.to_string()));
    }

    let sql = format!(
        "SELECT id, session_id, project, topic_key, title, content, memory_type, files, \
         created_at_epoch, updated_at_epoch, status \
         FROM memories WHERE {} ORDER BY updated_at_epoch DESC",
        conditions.join(" AND ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db_query::collect_rows(rows)
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
    let mut conditions = vec!["memories_fts MATCH ?1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(query.to_string()));

    let mut idx = 2;
    conditions.push("m.status = 'active'".to_string());

    if let Some(p) = project {
        conditions.push(format!("m.project = ?{idx}"));
        param_values.push(Box::new(p.to_string()));
        idx += 1;
    }
    if let Some(t) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
        idx += 1;
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content, \
         m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch, m.status \
         FROM memories m \
         JOIN memories_fts ON memories_fts.rowid = m.id \
         WHERE {} \
         ORDER BY ((-rank) * CASE WHEN m.memory_type IN ('decision','bugfix') THEN 1.5 ELSE 1.0 END) DESC \
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

/// LIKE fallback for short tokens.
pub fn search_memories_like(
    conn: &Connection,
    tokens: &[&str],
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Memory>> {
    if tokens.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = vec!["m.status = 'active'".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    for token in tokens {
        let like_pattern = format!("%{token}%");
        let cols = ["m.title", "m.content"];
        let token_clauses: Vec<String> = cols
            .iter()
            .map(|col| format!("{col} LIKE ?{idx}"))
            .collect();
        param_values.push(Box::new(like_pattern));
        conditions.push(format!("({})", token_clauses.join(" OR ")));
        idx += 1;
    }

    if let Some(p) = project {
        conditions.push(format!("m.project = ?{idx}"));
        param_values.push(Box::new(p.to_string()));
        idx += 1;
    }
    if let Some(t) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
        idx += 1;
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content, \
         m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch, m.status \
         FROM memories m \
         WHERE {} \
         ORDER BY m.updated_at_epoch DESC \
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

// --- Event CRUD ---

pub fn insert_event(
    conn: &Connection,
    session_id: &str,
    project: &str,
    event_type: &str,
    summary: &str,
    detail: Option<&str>,
    files: Option<&str>,
    exit_code: Option<i32>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO events \
         (session_id, project, event_type, summary, detail, files, exit_code, created_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![session_id, project, event_type, summary, detail, files, exit_code, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_session_events(conn: &Connection, session_id: &str) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, event_type, summary, detail, files, exit_code, \
         created_at_epoch \
         FROM events WHERE session_id = ?1 ORDER BY created_at_epoch ASC",
    )?;
    let rows = stmt.query_map(params![session_id], map_event_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn get_recent_events(conn: &Connection, project: &str, limit: i64) -> Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, event_type, summary, detail, files, exit_code, \
         created_at_epoch \
         FROM events WHERE project = ?1 ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], map_event_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn cleanup_old_events(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    let count = conn.execute(
        "DELETE FROM events WHERE created_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

pub fn archive_stale_memories(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    let count = conn.execute(
        "UPDATE memories SET status = 'archived' \
         WHERE status = 'active' AND updated_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

/// Count memories saved by Claude in this session (used by Stop hook fallback).
pub fn count_session_memories(conn: &Connection, session_id: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get distinct files modified in a session's events.
pub fn get_session_files_modified(conn: &Connection, session_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT files FROM events \
         WHERE session_id = ?1 AND event_type IN ('file_edit', 'file_create') AND files IS NOT NULL",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        let files_json: String = row.get(0)?;
        Ok(files_json)
    })?;

    let mut result = Vec::new();
    for row in rows {
        let files_json = row?;
        if let Ok(arr) = serde_json::from_str::<Vec<String>>(&files_json) {
            for f in arr {
                if !result.contains(&f) {
                    result.push(f);
                }
            }
        }
    }
    Ok(result)
}

/// Get event count for a session.
pub fn count_session_events(conn: &Connection, session_id: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

// --- Row Mappers ---

fn map_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        topic_key: row.get(3)?,
        title: row.get(4)?,
        text: row.get(5)?,
        memory_type: row.get(6)?,
        files: row.get(7)?,
        created_at_epoch: row.get(8)?,
        updated_at_epoch: row.get(9)?,
        status: row.get(10)?,
    })
}

fn map_event_row(row: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        event_type: row.get(3)?,
        summary: row.get(4)?,
        detail: row.get(5)?,
        files: row.get(6)?,
        exit_code: row.get(7)?,
        created_at_epoch: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_memory_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE memories (
                id INTEGER PRIMARY KEY,
                session_id TEXT,
                project TEXT NOT NULL,
                topic_key TEXT,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                files TEXT,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'active'
            );
            CREATE VIRTUAL TABLE memories_fts USING fts5(
                title, content,
                content='memories',
                content_rowid='id',
                tokenize='trigram'
            );
            CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, title, content)
                VALUES (new.id, new.title, new.content);
            END;
            CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, title, content)
                VALUES ('delete', old.id, old.title, old.content);
                INSERT INTO memories_fts(rowid, title, content)
                VALUES (new.id, new.title, new.content);
            END;
            CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, title, content)
                VALUES ('delete', old.id, old.title, old.content);
            END;
            CREATE TABLE events (
                id INTEGER PRIMARY KEY,
                session_id TEXT NOT NULL,
                project TEXT NOT NULL,
                event_type TEXT NOT NULL,
                summary TEXT NOT NULL,
                detail TEXT,
                files TEXT,
                exit_code INTEGER,
                created_at_epoch INTEGER NOT NULL
            );",
        )
        .unwrap();
    }

    #[test]
    fn test_memory_insert_and_query() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let id = insert_memory(
            &conn,
            Some("session-1"),
            "test/proj",
            None,
            "FTS5 supports CJK",
            "Switched from unicode61 to trigram tokenizer for Chinese text search.",
            "decision",
            Some(r#"["src/db.rs"]"#),
        )
        .unwrap();
        assert!(id > 0);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].title, "FTS5 supports CJK");
        assert_eq!(memories[0].memory_type, "decision");
    }

    #[test]
    fn test_topic_key_upsert() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let id1 = insert_memory(
            &conn,
            Some("s1"),
            "test/proj",
            Some("fts5-search-strategy"),
            "FTS5 trigram v1",
            "Initial implementation using trigram.",
            "decision",
            None,
        )
        .unwrap();

        let id2 = insert_memory(
            &conn,
            Some("s2"),
            "test/proj",
            Some("fts5-search-strategy"),
            "FTS5 trigram v2",
            "Added LIKE fallback for short tokens.",
            "decision",
            None,
        )
        .unwrap();

        // Same topic_key → update, not insert
        assert_eq!(id1, id2);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].title, "FTS5 trigram v2");
        assert!(memories[0].text.contains("LIKE fallback"));
    }

    #[test]
    fn test_memory_fts_search() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        insert_memory(
            &conn,
            Some("s1"),
            "proj",
            None,
            "FTS5 trigram tokenizer 支持 CJK",
            "Switched to trigram for Chinese search support.",
            "decision",
            None,
        )
        .unwrap();

        insert_memory(
            &conn,
            Some("s1"),
            "proj",
            None,
            "Auth middleware rewrite",
            "Rewrote auth middleware for compliance.",
            "architecture",
            None,
        )
        .unwrap();

        let results = search_memories_fts(&conn, "trigram", Some("proj"), None, 10, 0).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].title.contains("trigram"));
    }

    #[test]
    fn test_memory_like_fallback() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        insert_memory(
            &conn,
            Some("s1"),
            "proj",
            None,
            "DB schema migration",
            "Updated schema from v7 to v8.",
            "decision",
            None,
        )
        .unwrap();

        // "DB" is 2 chars → LIKE fallback
        let results = search_memories_like(&conn, &["DB"], Some("proj"), None, 10, 0).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].title.contains("DB"));
    }

    #[test]
    fn test_memory_type_filter() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        insert_memory(
            &conn,
            Some("s1"),
            "proj",
            None,
            "Bug: unicode61 fails CJK",
            "Root cause: unicode61 tokenizer doesn't segment Chinese.",
            "bugfix",
            None,
        )
        .unwrap();
        insert_memory(
            &conn,
            Some("s1"),
            "proj",
            None,
            "Use trigram tokenizer",
            "Decided to use trigram for CJK support.",
            "decision",
            None,
        )
        .unwrap();

        let bugs = get_memories_by_type(&conn, "proj", "bugfix", 10).unwrap();
        assert_eq!(bugs.len(), 1);
        assert!(bugs[0].title.contains("unicode61"));

        let decisions = get_memories_by_type(&conn, "proj", "decision", 10).unwrap();
        assert_eq!(decisions.len(), 1);
    }

    #[test]
    fn test_event_insert_and_query() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        insert_event(
            &conn,
            "session-1",
            "proj",
            "file_edit",
            "Edit src/db.rs",
            None,
            Some(r#"["src/db.rs"]"#),
            None,
        )
        .unwrap();
        insert_event(
            &conn,
            "session-1",
            "proj",
            "bash",
            "Run `cargo test` (exit 0)",
            None,
            None,
            Some(0),
        )
        .unwrap();

        let events = get_session_events(&conn, "session-1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "file_edit");
        assert_eq!(events[1].exit_code, Some(0));
    }

    #[test]
    fn test_cleanup_old_events() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let now = chrono::Utc::now().timestamp();
        let old = now - (31 * 86400);
        conn.execute(
            "INSERT INTO events (session_id, project, event_type, summary, created_at_epoch)
             VALUES ('s1', 'proj', 'file_edit', 'old edit', ?1)",
            params![old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (session_id, project, event_type, summary, created_at_epoch)
             VALUES ('s2', 'proj', 'file_edit', 'new edit', ?1)",
            params![now],
        )
        .unwrap();

        let deleted = cleanup_old_events(&conn, 30).unwrap();
        assert_eq!(deleted, 1);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_archive_stale_memories() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let now = chrono::Utc::now().timestamp();
        let old = now - (181 * 86400);
        conn.execute(
            "INSERT INTO memories (session_id, project, title, content, memory_type, \
             created_at_epoch, updated_at_epoch, status)
             VALUES ('s1', 'proj', 'old', 'old content', 'decision', ?1, ?1, 'active')",
            params![old],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (session_id, project, title, content, memory_type, \
             created_at_epoch, updated_at_epoch, status)
             VALUES ('s2', 'proj', 'new', 'new content', 'decision', ?1, ?1, 'active')",
            params![now],
        )
        .unwrap();

        let archived = archive_stale_memories(&conn, 180).unwrap();
        assert_eq!(archived, 1);

        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }
}
