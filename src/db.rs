use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: i64,
    pub memory_session_id: String,
    pub r#type: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub narrative: Option<String>,
    pub facts: Option<String>,
    pub concepts: Option<String>,
    pub files_read: Option<String>,
    pub files_modified: Option<String>,
    pub discovery_tokens: Option<i64>,
    pub created_at: String,
    pub created_at_epoch: i64,
    pub project: Option<String>,
    pub status: String,
    pub last_accessed_epoch: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: i64,
    pub memory_session_id: String,
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
    pub created_at: String,
    pub created_at_epoch: i64,
    pub project: Option<String>,
}

pub fn project_from_cwd(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

pub fn db_path() -> PathBuf {
    let data_dir = std::env::var("REMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".remem")
        });
    data_dir.join("remem.db")
}

pub fn open_db() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open database: {}", path.display()))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    ensure_core_schema(&conn)?;
    ensure_pending_table(&conn)?;
    ensure_schema_migrations(&conn)?;
    Ok(conn)
}

fn ensure_core_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sdk_sessions (
            id INTEGER PRIMARY KEY,
            content_session_id TEXT UNIQUE NOT NULL,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            user_prompt TEXT,
            started_at TEXT,
            started_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            prompt_counter INTEGER DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER
        );

        CREATE TABLE IF NOT EXISTS session_summaries (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            request TEXT,
            investigated TEXT,
            learned TEXT,
            completed TEXT,
            next_steps TEXT,
            notes TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS observations_fts USING fts5(
            title, subtitle, narrative, facts, concepts,
            content='observations',
            content_rowid='id'
        );

        CREATE TRIGGER IF NOT EXISTS observations_ai AFTER INSERT ON observations BEGIN
            INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
            VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
        END;

        CREATE TRIGGER IF NOT EXISTS observations_ad AFTER DELETE ON observations BEGIN
            INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
            VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
        END;

        CREATE TRIGGER IF NOT EXISTS observations_au AFTER UPDATE ON observations BEGIN
            INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
            VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
            INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
            VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
        END;"
    )?;
    Ok(())
}

fn ensure_pending_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pending_observations (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT,
            tool_response TEXT,
            cwd TEXT,
            created_at_epoch INTEGER NOT NULL
        )"
    )?;
    Ok(())
}

fn ensure_schema_migrations(conn: &Connection) -> Result<()> {
    for sql in &[
        "ALTER TABLE observations ADD COLUMN status TEXT DEFAULT 'active'",
        "ALTER TABLE observations ADD COLUMN last_accessed_epoch INTEGER",
        "ALTER TABLE session_summaries ADD COLUMN decisions TEXT",
        "ALTER TABLE session_summaries ADD COLUMN preferences TEXT",
    ] {
        if let Err(e) = conn.execute_batch(sql) {
            if !e.to_string().contains("duplicate column") {
                return Err(e.into());
            }
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_observations_status ON observations(status);
         CREATE INDEX IF NOT EXISTS idx_observations_project_status
           ON observations(project, status, created_at_epoch DESC);"
    )?;
    Ok(())
}

#[derive(Debug)]
pub struct PendingObservation {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub tool_name: String,
    pub tool_input: Option<String>,
    pub tool_response: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

pub fn enqueue_pending(
    conn: &Connection,
    session_id: &str,
    project: &str,
    tool_name: &str,
    tool_input: Option<&str>,
    tool_response: Option<&str>,
    cwd: Option<&str>,
) -> Result<i64> {
    let epoch = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO pending_observations \
         (session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![session_id, project, tool_name, tool_input, tool_response, cwd, epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn dequeue_pending(conn: &Connection, session_id: &str) -> Result<Vec<PendingObservation>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch \
         FROM pending_observations WHERE session_id = ?1 ORDER BY id ASC"
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok(PendingObservation {
            id: row.get(0)?,
            session_id: row.get(1)?,
            project: row.get(2)?,
            tool_name: row.get(3)?,
            tool_input: row.get(4)?,
            tool_response: row.get(5)?,
            cwd: row.get(6)?,
            created_at_epoch: row.get(7)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn delete_pending(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!("DELETE FROM pending_observations WHERE id IN ({})", placeholders.join(", "));
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    stmt.execute(refs.as_slice())?;
    Ok(())
}

pub fn count_pending(conn: &Connection, session_id: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn open_db_readonly() -> Result<Connection> {
    let path = db_path();
    let conn = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Failed to open database (readonly): {}", path.display()))?;
    Ok(conn)
}

pub fn query_observations(
    conn: &Connection,
    project: &str,
    types: &[&str],
    limit: i64,
) -> Result<Vec<Observation>> {
    if types.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = types.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect();
    let sql = format!(
        "SELECT id, memory_session_id, type, title, subtitle, narrative, \
         facts, concepts, files_read, files_modified, discovery_tokens, \
         created_at, created_at_epoch, status, last_accessed_epoch \
         FROM observations \
         WHERE project = ?1 AND type IN ({}) \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        placeholders.join(", "),
        types.len() + 2
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(project.to_string()));
    for t in types {
        param_values.push(Box::new(t.to_string()));
    }
    param_values.push(Box::new(limit));

    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(Observation {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            r#type: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            narrative: row.get(5)?,
            facts: row.get(6)?,
            concepts: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            discovery_tokens: row.get(10)?,
            created_at: row.get(11)?,
            created_at_epoch: row.get(12)?,
            project: Some(project.to_string()),
            status: row.get::<_, Option<String>>(13)?.unwrap_or_else(|| "active".to_string()),
            last_accessed_epoch: row.get(14)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn query_summaries(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
         next_steps, preferences, created_at, created_at_epoch \
         FROM session_summaries \
         WHERE project = ?1 \
         ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![project, limit], |row| {
        Ok(SessionSummary {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            request: row.get(2)?,
            completed: row.get(3)?,
            decisions: row.get(4)?,
            learned: row.get(5)?,
            next_steps: row.get(6)?,
            preferences: row.get(7)?,
            created_at: row.get(8)?,
            created_at_epoch: row.get(9)?,
            project: Some(project.to_string()),
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn search_observations_fts(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    let mut conditions = vec!["observations_fts MATCH ?1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(query.to_string()));

    let mut idx = 2;
    if let Some(p) = project {
        conditions.push(format!("o.project = ?{idx}"));
        param_values.push(Box::new(p.to_string()));
        idx += 1;
    }
    if let Some(t) = obs_type {
        conditions.push(format!("o.type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
        idx += 1;
    }
    if !include_stale {
        conditions.push("o.status = 'active'".to_string());
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT o.id, o.memory_session_id, o.type, o.title, o.subtitle, o.narrative, \
         o.facts, o.concepts, o.files_read, o.files_modified, o.discovery_tokens, \
         o.created_at, o.created_at_epoch, o.project, o.status, o.last_accessed_epoch \
         FROM observations o \
         JOIN observations_fts ON observations_fts.rowid = o.id \
         WHERE {} \
         ORDER BY (\
           rank * (1.0 + 0.5 * (strftime('%s','now') - o.created_at_epoch) / 2592000.0) \
           + CASE WHEN o.status = 'stale' THEN 1000.0 ELSE 0.0 END\
         ) \
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(Observation {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            r#type: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            narrative: row.get(5)?,
            facts: row.get(6)?,
            concepts: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            discovery_tokens: row.get(10)?,
            created_at: row.get(11)?,
            created_at_epoch: row.get(12)?,
            project: row.get(13)?,
            status: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "active".to_string()),
            last_accessed_epoch: row.get(15)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn get_observations_by_ids(
    conn: &Connection,
    ids: &[i64],
) -> Result<Vec<Observation>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT id, memory_session_id, type, title, subtitle, narrative, \
         facts, concepts, files_read, files_modified, discovery_tokens, \
         created_at, created_at_epoch, project, status, last_accessed_epoch \
         FROM observations WHERE id IN ({}) \
         ORDER BY created_at_epoch DESC",
        placeholders.join(", ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();

    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(Observation {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            r#type: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            narrative: row.get(5)?,
            facts: row.get(6)?,
            concepts: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            discovery_tokens: row.get(10)?,
            created_at: row.get(11)?,
            created_at_epoch: row.get(12)?,
            project: row.get(13)?,
            status: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "active".to_string()),
            last_accessed_epoch: row.get(15)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn insert_observation(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
    obs_type: &str,
    title: Option<&str>,
    subtitle: Option<&str>,
    narrative: Option<&str>,
    facts: Option<&str>,
    concepts: Option<&str>,
    files_read: Option<&str>,
    files_modified: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<i64> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    conn.execute(
        "INSERT INTO observations \
         (memory_session_id, project, type, title, subtitle, narrative, \
          facts, concepts, files_read, files_modified, prompt_number, \
          created_at, created_at_epoch, discovery_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            memory_session_id, project, obs_type, title, subtitle, narrative,
            facts, concepts, files_read, files_modified, prompt_number,
            created_at, created_at_epoch, discovery_tokens
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_summary(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
    request: Option<&str>,
    completed: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    next_steps: Option<&str>,
    preferences: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<i64> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    conn.execute(
        "INSERT INTO session_summaries \
         (memory_session_id, project, request, completed, decisions, learned, \
          next_steps, preferences, prompt_number, \
          created_at, created_at_epoch, discovery_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            memory_session_id, project, request, completed, decisions, learned,
            next_steps, preferences, prompt_number,
            created_at, created_at_epoch, discovery_tokens
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn mark_stale_by_files(
    conn: &Connection,
    new_obs_id: i64,
    project: &str,
    files_modified: &[String],
) -> Result<usize> {
    if files_modified.is_empty() {
        return Ok(0);
    }
    let files_json = serde_json::to_string(files_modified)?;
    let count = conn.execute(
        "UPDATE observations SET status = 'stale'
         WHERE id != ?1 AND project = ?2 AND status = 'active'
           AND id IN (
             SELECT DISTINCT o.id FROM observations o, json_each(o.files_modified) AS old_f
             WHERE o.id != ?1 AND o.project = ?2 AND o.status = 'active'
               AND o.files_modified IS NOT NULL AND length(o.files_modified) > 2
               AND old_f.value IN (SELECT value FROM json_each(?3))
           )",
        params![new_obs_id, project, files_json],
    )?;
    Ok(count)
}

/// Count active observations for a project.
pub fn count_active_observations(conn: &Connection, project: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM observations WHERE project = ?1 AND status IN ('active', 'stale')",
        params![project],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get oldest observations for compression (skipping the most recent `keep` items).
pub fn get_oldest_observations(
    conn: &Connection,
    project: &str,
    keep: i64,
    batch_size: i64,
) -> Result<Vec<Observation>> {
    // First get total count, then calculate how many old ones to take
    let total = count_active_observations(conn, project)?;
    let compressible = total - keep;
    if compressible <= 0 {
        return Ok(vec![]);
    }
    let take = compressible.min(batch_size);

    let mut stmt = conn.prepare(
        "SELECT id, memory_session_id, type, title, subtitle, narrative, \
         facts, concepts, files_read, files_modified, discovery_tokens, \
         created_at, created_at_epoch, status, last_accessed_epoch \
         FROM observations \
         WHERE project = ?1 AND status IN ('active', 'stale') \
         ORDER BY created_at_epoch ASC \
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![project, take], |row| {
        Ok(Observation {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            r#type: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            narrative: row.get(5)?,
            facts: row.get(6)?,
            concepts: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            discovery_tokens: row.get(10)?,
            created_at: row.get(11)?,
            created_at_epoch: row.get(12)?,
            project: Some(project.to_string()),
            status: row.get(13)?,
            last_accessed_epoch: row.get(14)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

/// Mark observations as compressed (they won't appear in context loading).
pub fn mark_observations_compressed(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations SET status = 'compressed' WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let count = stmt.execute(refs.as_slice())?;
    Ok(count)
}

pub fn update_last_accessed(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    let placeholders: Vec<String> = (2..=ids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations SET last_accessed_epoch = ?1 WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(now));
    for id in ids {
        param_values.push(Box::new(*id));
    }
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    stmt.execute(refs.as_slice())?;
    Ok(())
}

pub fn upsert_session(
    conn: &Connection,
    content_session_id: &str,
    project: &str,
    user_prompt: Option<&str>,
) -> Result<String> {
    let now = chrono::Utc::now();
    let started_at = now.to_rfc3339();
    let started_at_epoch = now.timestamp();
    let memory_session_id = format!("mem-{}", truncate_str(content_session_id, 8));

    conn.execute(
        "INSERT INTO sdk_sessions \
         (content_session_id, memory_session_id, project, user_prompt, \
          started_at, started_at_epoch, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active') \
         ON CONFLICT(content_session_id) DO UPDATE SET \
         prompt_counter = prompt_counter + 1",
        params![
            content_session_id, memory_session_id, project, user_prompt,
            started_at, started_at_epoch
        ],
    )?;

    let mid: String = conn.query_row(
        "SELECT memory_session_id FROM sdk_sessions WHERE content_session_id = ?1",
        params![content_session_id],
        |row| row.get(0),
    )?;
    Ok(mid)
}

pub fn get_timeline_around(
    conn: &Connection,
    anchor_id: i64,
    depth_before: i64,
    depth_after: i64,
    project: Option<&str>,
) -> Result<Vec<Observation>> {
    let anchor: Observation = conn.query_row(
        "SELECT id, memory_session_id, type, title, subtitle, narrative, \
         facts, concepts, files_read, files_modified, discovery_tokens, \
         created_at, created_at_epoch, project, status, last_accessed_epoch \
         FROM observations WHERE id = ?1",
        params![anchor_id],
        |row| {
            Ok(Observation {
                id: row.get(0)?,
                memory_session_id: row.get(1)?,
                r#type: row.get(2)?,
                title: row.get(3)?,
                subtitle: row.get(4)?,
                narrative: row.get(5)?,
                facts: row.get(6)?,
                concepts: row.get(7)?,
                files_read: row.get(8)?,
                files_modified: row.get(9)?,
                discovery_tokens: row.get(10)?,
                created_at: row.get(11)?,
                created_at_epoch: row.get(12)?,
                project: row.get(13)?,
                status: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "active".to_string()),
                last_accessed_epoch: row.get(15)?,
            })
        },
    )?;

    let epoch = anchor.created_at_epoch;
    let mut project_filter = String::new();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(p) = project {
        project_filter = " AND project = ?3".to_string();
        param_values.push(Box::new(p.to_string()));
    }

    let before_sql = format!(
        "SELECT id, memory_session_id, type, title, subtitle, narrative, \
         facts, concepts, files_read, files_modified, discovery_tokens, \
         created_at, created_at_epoch, project, status, last_accessed_epoch \
         FROM observations \
         WHERE created_at_epoch < ?1{} \
         ORDER BY created_at_epoch DESC LIMIT ?2",
        project_filter
    );

    let after_sql = format!(
        "SELECT id, memory_session_id, type, title, subtitle, narrative, \
         facts, concepts, files_read, files_modified, discovery_tokens, \
         created_at, created_at_epoch, project, status, last_accessed_epoch \
         FROM observations \
         WHERE created_at_epoch > ?1{} \
         ORDER BY created_at_epoch ASC LIMIT ?2",
        project_filter
    );

    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<Observation> {
        Ok(Observation {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            r#type: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            narrative: row.get(5)?,
            facts: row.get(6)?,
            concepts: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            discovery_tokens: row.get(10)?,
            created_at: row.get(11)?,
            created_at_epoch: row.get(12)?,
            project: row.get(13)?,
            status: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "active".to_string()),
            last_accessed_epoch: row.get(15)?,
        })
    };

    let mut result = Vec::new();

    // Before
    {
        let mut stmt = conn.prepare(&before_sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(epoch),
            Box::new(depth_before),
        ];
        if let Some(p) = project {
            params_vec.push(Box::new(p.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), map_row)?;
        for row in rows {
            result.push(row?);
        }
    }

    result.push(anchor);

    // After
    {
        let mut stmt = conn.prepare(&after_sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(epoch),
            Box::new(depth_after),
        ];
        if let Some(p) = project {
            params_vec.push(Box::new(p.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), map_row)?;
        for row in rows {
            result.push(row?);
        }
    }

    result.sort_by_key(|o| o.created_at_epoch);
    Ok(result)
}
