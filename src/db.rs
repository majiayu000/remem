// Re-export query functions so callers can still use `db::query_observations` etc.
pub use crate::db_query::*;

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
           ON observations(project, status, created_at_epoch DESC);

         CREATE TABLE IF NOT EXISTS summarize_cooldown (
             project TEXT PRIMARY KEY,
             last_summarize_epoch INTEGER NOT NULL,
             last_message_hash TEXT
         );"
    )?;
    Ok(())
}

// --- Summarize rate limiting ---

/// 检查项目是否在冷却期内。返回 true = 应该跳过。
pub fn is_summarize_on_cooldown(conn: &Connection, project: &str, cooldown_secs: i64) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let result: Option<i64> = conn.query_row(
        "SELECT last_summarize_epoch FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    ).ok();

    match result {
        Some(last_epoch) => Ok(now - last_epoch < cooldown_secs),
        None => Ok(false),
    }
}

/// 检查 message hash 是否与上次相同。返回 true = 重复消息，应该跳过。
pub fn is_duplicate_message(conn: &Connection, project: &str, message_hash: &str) -> Result<bool> {
    let result: Option<String> = conn.query_row(
        "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    ).ok().flatten();

    match result {
        Some(prev_hash) => Ok(prev_hash == message_hash),
        None => Ok(false),
    }
}

/// 记录本次 summarize 的时间和 message hash。
pub fn record_summarize(conn: &Connection, project: &str, message_hash: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?2,
           last_message_hash = ?3",
        params![project, now, message_hash],
    )?;
    Ok(())
}

// --- 数据清理 ---

/// 删除无对应 observation 的旧版 mem-* summary。
pub fn cleanup_orphan_summaries(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM session_summaries
         WHERE memory_session_id LIKE 'mem-%'
           AND memory_session_id NOT IN (
             SELECT DISTINCT memory_session_id FROM observations
           )",
        [],
    )?;
    Ok(count)
}

/// 删除同 session 的重复 summary，只保留最新的一条。
pub fn cleanup_duplicate_summaries(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM session_summaries
         WHERE id NOT IN (
           SELECT MAX(id)
           FROM session_summaries
           GROUP BY memory_session_id, project
         )",
        [],
    )?;
    Ok(count)
}

/// 清理已处理但残留的 pending observations（超过 1 小时未处理的）。
pub fn cleanup_stale_pending(conn: &Connection) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - 3600;
    let count = conn.execute(
        "DELETE FROM pending_observations WHERE created_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
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

