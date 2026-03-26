use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db;

// Re-export search and promote functions so existing callers don't break.
pub use crate::memory_promote::{promote_summary_to_memories, slugify_for_topic};
pub use crate::memory_search::{search_memories_fts, search_memories_like};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String {
    "project".to_string()
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
    insert_memory_with_branch(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        None,
    )
}

pub fn insert_memory_with_branch(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
) -> Result<i64> {
    insert_memory_full(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        branch,
        "project",
        None,
    )
}

pub fn insert_memory_full(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    created_at_override: Option<i64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let created_at = created_at_override.unwrap_or(now);

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
                     memory_type = ?4, files = ?5, updated_at_epoch = ?6, branch = ?7, \
                     scope = ?8 WHERE id = ?9",
                    params![
                        session_id,
                        title,
                        content,
                        memory_type,
                        files,
                        now,
                        branch,
                        scope,
                        id
                    ],
                )?;
                // Refresh entity links on upsert
                let entities = crate::entity::extract_entities(title, content);
                if !entities.is_empty() {
                    if let Err(e) = crate::entity::link_entities(conn, id, &entities) {
                        crate::log::warn("memory", &format!("entity link refresh failed: {}", e));
                    }
                }
                return Ok(id);
            }
        }
    }

    conn.execute(
        "INSERT INTO memories \
         (session_id, project, topic_key, title, content, memory_type, files, \
          created_at_epoch, updated_at_epoch, status, branch, scope) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11)",
        params![
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            created_at,
            now,
            branch,
            scope
        ],
    )?;
    let id = conn.last_insert_rowid();

    // Auto-link entities on every memory insert
    let entities = crate::entity::extract_entities(title, content);
    if !entities.is_empty() {
        if let Err(e) = crate::entity::link_entities(conn, id, &entities) {
            crate::log::warn(
                "memory",
                &format!("entity link failed for id={}: {}", id, e),
            );
        }
    }

    Ok(id)
}

pub fn get_recent_memories(conn: &Connection, project: &str, limit: i64) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE (project = ?1 OR scope = 'global') AND status = 'active' \
         ORDER BY updated_at_epoch DESC LIMIT ?2",
        MEMORY_COLS,
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project, limit], map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

pub fn get_memories_by_type(
    conn: &Connection,
    project: &str,
    memory_type: &str,
    limit: i64,
) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {} FROM memories \
         WHERE (project = ?1 OR scope = 'global') AND memory_type = ?2 AND status = 'active' \
         ORDER BY updated_at_epoch DESC LIMIT ?3",
        MEMORY_COLS,
    );
    let mut stmt = conn.prepare(&sql)?;
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
        "SELECT {} FROM memories WHERE {} ORDER BY updated_at_epoch DESC",
        MEMORY_COLS,
        conditions.join(" AND ")
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

pub fn count_session_memories(conn: &Connection, session_id: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

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

pub fn count_session_events(conn: &Connection, session_id: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

// --- Row Mappers ---

pub fn map_memory_row_pub(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    map_memory_row(row)
}

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
        branch: row.get(11)?,
        scope: row
            .get::<_, Option<String>>(12)?
            .unwrap_or_else(|| "project".to_string()),
    })
}

pub const MEMORY_COLS: &str = "id, session_id, project, topic_key, title, content, memory_type, \
                              files, created_at_epoch, updated_at_epoch, status, branch, scope";

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

// --- Test Helper (shared with memory_promote tests) ---

#[cfg(test)]
pub mod tests_helper {
    use rusqlite::Connection;

    pub fn setup_memory_schema(conn: &Connection) {
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
                status TEXT NOT NULL DEFAULT 'active',
                branch TEXT,
                scope TEXT DEFAULT 'project'
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
            );
            CREATE TABLE IF NOT EXISTS entities (
                id INTEGER PRIMARY KEY,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                entity_type TEXT,
                mention_count INTEGER DEFAULT 1,
                created_at_epoch INTEGER NOT NULL DEFAULT 0,
                UNIQUE(canonical_name)
            );
            CREATE TABLE IF NOT EXISTS memory_entities (
                memory_id INTEGER NOT NULL,
                entity_id INTEGER NOT NULL,
                PRIMARY KEY(memory_id, entity_id)
            );",
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tests_helper::setup_memory_schema;

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

        assert_eq!(id1, id2);
        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].title, "FTS5 trigram v2");
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

        assert_eq!(
            get_memories_by_type(&conn, "proj", "bugfix", 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            get_memories_by_type(&conn, "proj", "decision", 10)
                .unwrap()
                .len(),
            1
        );
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

        assert_eq!(cleanup_old_events(&conn, 30).unwrap(), 1);
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

        assert_eq!(archive_stale_memories(&conn, 180).unwrap(), 1);
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }

    #[test]
    fn test_created_at_override() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let custom_epoch: i64 = 1_700_000_000; // 2023-11-14
        let id = insert_memory_full(
            &conn,
            Some("s1"),
            "proj",
            None,
            "Old event",
            "Something from the past",
            "discovery",
            None,
            None,
            "project",
            Some(custom_epoch),
        )
        .unwrap();

        let row: (i64, i64) = conn
            .query_row(
                "SELECT created_at_epoch, updated_at_epoch FROM memories WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();

        assert_eq!(row.0, custom_epoch, "created_at_epoch should use override");
        assert_ne!(
            row.1, custom_epoch,
            "updated_at_epoch should use current time"
        );
    }

    #[test]
    fn test_created_at_default_when_no_override() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let before = chrono::Utc::now().timestamp();
        let id = insert_memory_full(
            &conn,
            Some("s1"),
            "proj",
            None,
            "Recent event",
            "Something now",
            "discovery",
            None,
            None,
            "project",
            None,
        )
        .unwrap();
        let after = chrono::Utc::now().timestamp();

        let created: i64 = conn
            .query_row(
                "SELECT created_at_epoch FROM memories WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();

        assert!(
            created >= before && created <= after,
            "created_at_epoch should be current time when no override"
        );
    }

    // promote tests are in memory_promote::tests
}
