// Re-export query functions so callers can still use `db::query_observations` etc.
pub use crate::db_query::*;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
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

fn project_label_from_path(path: &std::path::Path) -> String {
    let components: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n),
            _ => None,
        })
        .collect();
    match components.len() {
        0 => path.to_string_lossy().to_string(),
        1 => components[0].to_string_lossy().to_string(),
        n => format!(
            "{}/{}",
            components[n - 2].to_string_lossy(),
            components[n - 1].to_string_lossy()
        ),
    }
}

fn canonical_project_path(cwd: &str) -> PathBuf {
    let path = std::path::Path::new(cwd);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    std::fs::canonicalize(&abs).unwrap_or_else(|e| {
        crate::log::warn(
            "db",
            &format!("canonicalize {:?} failed (using abs): {}", abs, e),
        );
        abs
    })
}

/// Build a stable project key from cwd.
/// Format: "<last2>@<hash12>", where hash is derived from canonical absolute path.
/// Example: "tools/remem@b7f8a1d44c2e"
pub fn project_from_cwd(cwd: &str) -> String {
    let canonical = canonical_project_path(cwd);
    let label = project_label_from_path(&canonical);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.to_string_lossy().hash(&mut hasher);
    let suffix = hasher.finish() & 0x0000_FFFF_FFFF_FFFF;
    format!("{label}@{suffix:012x}")
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

/// Current schema version — bump when adding migrations.
const SCHEMA_VERSION: i64 = 4;

pub fn open_db() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open database: {}", path.display()))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        ensure_core_schema(&conn)?;
        ensure_pending_table(&conn)?;
        ensure_schema_migrations(&conn)?;
        conn.execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION))?;
    }

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
            completed TEXT,
            decisions TEXT,
            learned TEXT,
            next_steps TEXT,
            preferences TEXT,
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
            created_at_epoch INTEGER NOT NULL,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        )",
    )?;
    Ok(())
}

fn ensure_schema_migrations(conn: &Connection) -> Result<()> {
    let migrations: &[(&str, &str, &str)] = &[
        ("observations", "status", "ALTER TABLE observations ADD COLUMN status TEXT DEFAULT 'active'"),
        ("observations", "last_accessed_epoch", "ALTER TABLE observations ADD COLUMN last_accessed_epoch INTEGER"),
        ("session_summaries", "decisions", "ALTER TABLE session_summaries ADD COLUMN decisions TEXT"),
        ("session_summaries", "preferences", "ALTER TABLE session_summaries ADD COLUMN preferences TEXT"),
        ("pending_observations", "lease_owner", "ALTER TABLE pending_observations ADD COLUMN lease_owner TEXT"),
        ("pending_observations", "lease_expires_epoch", "ALTER TABLE pending_observations ADD COLUMN lease_expires_epoch INTEGER"),
    ];
    for (table, col, sql) in migrations {
        if !column_exists(conn, table, col)? {
            conn.execute_batch(sql)?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_observations_status ON observations(status);
         CREATE INDEX IF NOT EXISTS idx_observations_project_status
           ON observations(project, status, created_at_epoch DESC);
         CREATE INDEX IF NOT EXISTS idx_pending_session_lease
           ON pending_observations(session_id, lease_expires_epoch, id);
         CREATE INDEX IF NOT EXISTS idx_pending_project_lease
           ON pending_observations(project, lease_expires_epoch, created_at_epoch);

         CREATE TABLE IF NOT EXISTS summarize_cooldown (
             project TEXT PRIMARY KEY,
             last_summarize_epoch INTEGER NOT NULL,
             last_message_hash TEXT
         );

         CREATE TABLE IF NOT EXISTS summarize_locks (
             project TEXT PRIMARY KEY,
             lock_epoch INTEGER NOT NULL
         );

         CREATE TABLE IF NOT EXISTS ai_usage_events (
             id INTEGER PRIMARY KEY,
             created_at TEXT NOT NULL,
             created_at_epoch INTEGER NOT NULL,
             project TEXT,
             operation TEXT NOT NULL,
             executor TEXT NOT NULL,
             model TEXT,
             input_tokens INTEGER NOT NULL,
             output_tokens INTEGER NOT NULL,
             total_tokens INTEGER NOT NULL,
             estimated_cost_usd REAL NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_ai_usage_created
           ON ai_usage_events(created_at_epoch DESC);
         CREATE INDEX IF NOT EXISTS idx_ai_usage_project_created
           ON ai_usage_events(project, created_at_epoch DESC);",
    )?;
    migrate_session_summaries_v4(conn)?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_session_summaries_v4(conn: &Connection) -> Result<()> {
    let has_investigated = column_exists(conn, "session_summaries", "investigated")?;
    let has_notes = column_exists(conn, "session_summaries", "notes")?;
    if !has_investigated && !has_notes {
        return Ok(());
    }

    let completed_expr = if has_investigated {
        "COALESCE(completed, investigated)"
    } else {
        "completed"
    };
    let preferences_expr = if has_notes {
        "COALESCE(preferences, notes)"
    } else {
        "preferences"
    };

    let sql = format!(
        "BEGIN IMMEDIATE;
         DROP TABLE IF EXISTS session_summaries_v4;
         CREATE TABLE session_summaries_v4 (
             id INTEGER PRIMARY KEY,
             memory_session_id TEXT NOT NULL,
             project TEXT,
             request TEXT,
             completed TEXT,
             decisions TEXT,
             learned TEXT,
             next_steps TEXT,
             preferences TEXT,
             prompt_number INTEGER,
             created_at TEXT,
             created_at_epoch INTEGER,
             discovery_tokens INTEGER DEFAULT 0
         );
         INSERT INTO session_summaries_v4
             (id, memory_session_id, project, request, completed, decisions, learned,
              next_steps, preferences, prompt_number, created_at, created_at_epoch, discovery_tokens)
         SELECT id, memory_session_id, project, request, {completed_expr}, decisions, learned,
                next_steps, {preferences_expr}, prompt_number, created_at, created_at_epoch, discovery_tokens
         FROM session_summaries;
         DROP TABLE session_summaries;
         ALTER TABLE session_summaries_v4 RENAME TO session_summaries;
         CREATE INDEX IF NOT EXISTS idx_session_summaries_project_created
           ON session_summaries(project, created_at_epoch DESC);
         COMMIT;"
    );
    conn.execute_batch(&sql)?;
    Ok(())
}

// --- Summarize rate limiting ---

/// 检查项目是否在冷却期内。返回 true = 应该跳过。
pub fn is_summarize_on_cooldown(
    conn: &Connection,
    project: &str,
    cooldown_secs: i64,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT last_summarize_epoch FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    );

    match result {
        Ok(last_epoch) => Ok(now - last_epoch < cooldown_secs),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// 检查 message hash 是否与上次相同。返回 true = 重复消息，应该跳过。
pub fn is_duplicate_message(conn: &Connection, project: &str, message_hash: &str) -> Result<bool> {
    let result: rusqlite::Result<Option<String>> = conn.query_row(
        "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    );

    match result {
        Ok(Some(prev_hash)) => Ok(prev_hash == message_hash),
        Ok(None) => Ok(false),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Try to acquire a short-lived summarize lock for one project.
/// Returns false when another worker currently owns a non-expired lock.
pub fn try_acquire_summarize_lock(
    conn: &mut Connection,
    project: &str,
    lock_secs: i64,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let lock_secs = lock_secs.max(1);
    let tx = conn.transaction()?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT lock_epoch FROM summarize_locks WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(epoch) = existing {
        if now - epoch < lock_secs {
            tx.rollback()?;
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT INTO summarize_locks (project, lock_epoch)
         VALUES (?1, ?2)
         ON CONFLICT(project) DO UPDATE SET lock_epoch = ?2",
        params![project, now],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn release_summarize_lock(conn: &Connection, project: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM summarize_locks WHERE project = ?1",
        params![project],
    )?;
    Ok(())
}

/// 原子替换 summary + 更新 summarize 冷却/去重 gate。
/// 返回值为被替换掉的旧 summary 条数。
pub fn finalize_summarize(
    conn: &mut Connection,
    memory_session_id: &str,
    project: &str,
    message_hash: &str,
    request: Option<&str>,
    completed: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    next_steps: Option<&str>,
    preferences: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<usize> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    let tx = conn.transaction()?;
    let deleted = tx.execute(
        "DELETE FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
        params![memory_session_id, project],
    )?;
    tx.execute(
        "INSERT INTO session_summaries \
         (memory_session_id, project, request, completed, decisions, learned, \
          next_steps, preferences, prompt_number, created_at, created_at_epoch, discovery_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            memory_session_id,
            project,
            request,
            completed,
            decisions,
            learned,
            next_steps,
            preferences,
            prompt_number,
            created_at,
            created_at_epoch,
            discovery_tokens
        ],
    )?;
    tx.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?2,
           last_message_hash = ?3",
        params![project, created_at_epoch, message_hash],
    )?;
    tx.commit()?;
    Ok(deleted)
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
    let now = chrono::Utc::now().timestamp();
    let count = conn.execute(
        "DELETE FROM pending_observations
         WHERE created_at_epoch < ?1
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?2)",
        params![cutoff, now],
    )?;
    Ok(count)
}

/// 清理已压缩超过 ttl_days 天的旧 observations。
pub fn cleanup_expired_compressed(conn: &Connection, ttl_days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (ttl_days * 86400);
    let count = conn.execute(
        "DELETE FROM observations WHERE status = 'compressed' AND created_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

#[derive(Debug)]
#[allow(dead_code)]
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
         (session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch, lease_owner, lease_expires_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)",
        params![session_id, project, tool_name, tool_input, tool_response, cwd, epoch],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Claim a pending batch for processing with a short lease.
/// Claimed rows must be either deleted on success or released on failure.
pub fn claim_pending(
    conn: &Connection,
    session_id: &str,
    limit: usize,
    lease_owner: &str,
    lease_secs: i64,
) -> Result<Vec<PendingObservation>> {
    let now = chrono::Utc::now().timestamp();
    let lease_expires = now + lease_secs.max(1);
    conn.execute(
        "UPDATE pending_observations
         SET lease_owner = ?1, lease_expires_epoch = ?2
         WHERE id IN (
             SELECT id FROM pending_observations
             WHERE session_id = ?3
               AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)
             ORDER BY id ASC
             LIMIT ?5
         )
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?4)",
        params![lease_owner, lease_expires, session_id, now, limit as i64],
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch \
         FROM pending_observations
         WHERE session_id = ?1 AND lease_owner = ?2
         ORDER BY id ASC"
    )?;
    let rows = stmt.query_map(params![session_id, lease_owner], |row| {
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

pub fn release_pending_claims(conn: &Connection, lease_owner: &str) -> Result<usize> {
    let count = conn.execute(
        "UPDATE pending_observations
         SET lease_owner = NULL, lease_expires_epoch = NULL
         WHERE lease_owner = ?1",
        params![lease_owner],
    )?;
    Ok(count)
}

pub fn delete_pending_claimed(conn: &Connection, lease_owner: &str, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = (2..=ids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "DELETE FROM pending_observations WHERE lease_owner = ?1 AND id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(lease_owner.to_string()));
    for id in ids {
        param_values.push(Box::new(*id));
    }
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let count = stmt.execute(refs.as_slice())?;
    Ok(count)
}

/// Get distinct session IDs with stale pending observations (older than age_secs).
pub fn get_stale_pending_sessions(
    conn: &Connection,
    project: &str,
    age_secs: i64,
) -> Result<Vec<String>> {
    let cutoff = chrono::Utc::now().timestamp() - age_secs;
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT session_id FROM pending_observations \
         WHERE project = ?1 AND created_at_epoch < ?2 \
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?3)",
    )?;
    let rows = stmt.query_map(params![project, cutoff, now], |row| row.get(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn count_pending(conn: &Connection, session_id: &str) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations
         WHERE session_id = ?1
           AND (lease_owner IS NULL OR lease_expires_epoch IS NULL OR lease_expires_epoch < ?2)",
        params![session_id, now],
        |row| row.get(0),
    )?;
    Ok(count)
}

#[derive(Debug, Clone)]
pub struct AiUsageEvent {
    pub created_at: String,
    pub project: Option<String>,
    pub operation: String,
    pub executor: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct DailyAiUsage {
    pub day: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone)]
pub struct AiUsageTotals {
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub estimated_cost_usd: f64,
}

pub fn record_ai_usage(
    conn: &Connection,
    project: Option<&str>,
    operation: &str,
    executor: &str,
    model: Option<&str>,
    input_tokens: i64,
    output_tokens: i64,
    estimated_cost_usd: f64,
) -> Result<i64> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();
    let total_tokens = input_tokens + output_tokens;
    conn.execute(
        "INSERT INTO ai_usage_events
         (created_at, created_at_epoch, project, operation, executor, model,
          input_tokens, output_tokens, total_tokens, estimated_cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            created_at,
            created_at_epoch,
            project,
            operation,
            executor,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            estimated_cost_usd
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn usage_cutoff_epoch(days: i64) -> i64 {
    chrono::Utc::now().timestamp() - days.max(1) * 86400
}

pub fn query_ai_usage_events_since(
    conn: &Connection,
    from_epoch: i64,
    limit: i64,
    project: Option<&str>,
) -> Result<Vec<AiUsageEvent>> {
    let mut conditions = vec!["created_at_epoch >= ?1".to_string()];
    let mut params_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    params_values.push(Box::new(from_epoch));

    let mut idx = 2;
    if let Some(p) = project {
        conditions.push(format!("project = ?{idx}"));
        params_values.push(Box::new(p.to_string()));
        idx += 1;
    }
    params_values.push(Box::new(limit.max(1)));
    let sql = format!(
        "SELECT created_at, project, operation, executor, model,
                input_tokens, output_tokens, total_tokens, estimated_cost_usd
         FROM ai_usage_events
         WHERE {}
         ORDER BY created_at_epoch DESC
         LIMIT ?{}",
        conditions.join(" AND "),
        idx
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = params_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(AiUsageEvent {
            created_at: row.get(0)?,
            project: row.get(1)?,
            operation: row.get(2)?,
            executor: row.get(3)?,
            model: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            total_tokens: row.get(7)?,
            estimated_cost_usd: row.get(8)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn query_ai_usage_events(
    conn: &Connection,
    days: i64,
    limit: i64,
    project: Option<&str>,
) -> Result<Vec<AiUsageEvent>> {
    query_ai_usage_events_since(conn, usage_cutoff_epoch(days), limit, project)
}

pub fn query_ai_usage_daily_since(
    conn: &Connection,
    from_epoch: i64,
    project: Option<&str>,
) -> Result<Vec<DailyAiUsage>> {
    let mut conditions = vec!["created_at_epoch >= ?1".to_string()];
    let mut params_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    params_values.push(Box::new(from_epoch));

    if let Some(p) = project {
        conditions.push("project = ?2".to_string());
        params_values.push(Box::new(p.to_string()));
    }
    let sql = format!(
        "SELECT date(created_at_epoch, 'unixepoch', 'localtime') AS day,
                COUNT(*) AS calls,
                COALESCE(SUM(input_tokens), 0) AS input_tokens,
                COALESCE(SUM(output_tokens), 0) AS output_tokens,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COALESCE(SUM(estimated_cost_usd), 0.0) AS estimated_cost_usd
         FROM ai_usage_events
         WHERE {}
         GROUP BY day
         ORDER BY day DESC",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = params_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(DailyAiUsage {
            day: row.get(0)?,
            calls: row.get(1)?,
            input_tokens: row.get(2)?,
            output_tokens: row.get(3)?,
            total_tokens: row.get(4)?,
            estimated_cost_usd: row.get(5)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn query_ai_usage_daily(
    conn: &Connection,
    days: i64,
    project: Option<&str>,
) -> Result<Vec<DailyAiUsage>> {
    query_ai_usage_daily_since(conn, usage_cutoff_epoch(days), project)
}

pub fn query_ai_usage_totals_since(
    conn: &Connection,
    from_epoch: i64,
    project: Option<&str>,
) -> Result<AiUsageTotals> {
    let mut conditions = vec!["created_at_epoch >= ?1".to_string()];
    let mut params_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    params_values.push(Box::new(from_epoch));

    if let Some(p) = project {
        conditions.push("project = ?2".to_string());
        params_values.push(Box::new(p.to_string()));
    }
    let sql = format!(
        "SELECT COUNT(*),
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM ai_usage_events
         WHERE {}",
        conditions.join(" AND ")
    );
    let refs: Vec<&dyn rusqlite::types::ToSql> = params_values.iter().map(|b| b.as_ref()).collect();
    let totals = conn.query_row(&sql, refs.as_slice(), |row| {
        Ok(AiUsageTotals {
            calls: row.get(0)?,
            input_tokens: row.get(1)?,
            output_tokens: row.get(2)?,
            total_tokens: row.get(3)?,
            estimated_cost_usd: row.get(4)?,
        })
    })?;
    Ok(totals)
}

pub fn query_ai_usage_totals(
    conn: &Connection,
    days: i64,
    project: Option<&str>,
) -> Result<AiUsageTotals> {
    query_ai_usage_totals_since(conn, usage_cutoff_epoch(days), project)
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
            memory_session_id,
            project,
            obs_type,
            title,
            subtitle,
            narrative,
            facts,
            concepts,
            files_read,
            files_modified,
            prompt_number,
            created_at,
            created_at_epoch,
            discovery_tokens
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
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
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
            content_session_id,
            memory_session_id,
            project,
            user_prompt,
            started_at,
            started_at_epoch
        ],
    )?;

    let mid: String = conn.query_row(
        "SELECT memory_session_id FROM sdk_sessions WHERE content_session_id = ?1",
        params![content_session_id],
        |row| row.get(0),
    )?;
    Ok(mid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rusqlite::Connection;

    fn setup_summary_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE session_summaries (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                request TEXT,
                completed TEXT,
                decisions TEXT,
                learned TEXT,
                next_steps TEXT,
                preferences TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0
            );
            CREATE TABLE summarize_cooldown (
                project TEXT PRIMARY KEY,
                last_summarize_epoch INTEGER NOT NULL,
                last_message_hash TEXT
            );",
        )?;
        Ok(())
    }

    fn setup_pending_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE pending_observations (
                id INTEGER PRIMARY KEY,
                session_id TEXT NOT NULL,
                project TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                tool_input TEXT,
                tool_response TEXT,
                cwd TEXT,
                created_at_epoch INTEGER NOT NULL,
                lease_owner TEXT,
                lease_expires_epoch INTEGER
            );",
        )?;
        Ok(())
    }

    fn setup_usage_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE ai_usage_events (
                id INTEGER PRIMARY KEY,
                created_at TEXT NOT NULL,
                created_at_epoch INTEGER NOT NULL,
                project TEXT,
                operation TEXT NOT NULL,
                executor TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                estimated_cost_usd REAL NOT NULL
            );",
        )?;
        Ok(())
    }

    fn setup_legacy_summary_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE session_summaries (
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
                discovery_tokens INTEGER DEFAULT 0,
                decisions TEXT,
                preferences TEXT
            );",
        )?;
        Ok(())
    }

    #[test]
    fn migrate_legacy_summary_columns_to_v4_shape() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_legacy_summary_schema(&conn)?;
        conn.execute(
            "INSERT INTO session_summaries
             (memory_session_id, project, request, investigated, learned, next_steps, notes,
              created_at, created_at_epoch, discovery_tokens, decisions, preferences)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
            params![
                "mem-legacy",
                "proj",
                "req",
                "legacy-completed",
                "learned",
                "next",
                "legacy-pref",
                "2026-01-01T00:00:00Z",
                1_i64,
                5_i64,
                "decision"
            ],
        )?;

        migrate_session_summaries_v4(&conn)?;

        assert!(!column_exists(&conn, "session_summaries", "investigated")?);
        assert!(!column_exists(&conn, "session_summaries", "notes")?);

        let completed: String = conn.query_row(
            "SELECT completed FROM session_summaries WHERE memory_session_id = 'mem-legacy'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(completed, "legacy-completed");

        let preferences: String = conn.query_row(
            "SELECT preferences FROM session_summaries WHERE memory_session_id = 'mem-legacy'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(preferences, "legacy-pref");
        Ok(())
    }

    #[test]
    fn finalize_summarize_replaces_in_single_commit() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_summary_schema(&conn)?;
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch, discovery_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["mem-1", "proj", "old", "2026-01-01T00:00:00Z", 1_i64, 10_i64],
        )?;

        let deleted = finalize_summarize(
            &mut conn,
            "mem-1",
            "proj",
            "hash-1",
            Some("new"),
            Some("done"),
            Some("decision"),
            Some("learned"),
            Some("next"),
            Some("pref"),
            None,
            99,
        )?;
        assert_eq!(deleted, 1);

        let req: String = conn.query_row(
            "SELECT request FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
            params!["mem-1", "proj"],
            |r| r.get(0),
        )?;
        assert_eq!(req, "new");

        let hash: String = conn.query_row(
            "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
            params!["proj"],
            |r| r.get(0),
        )?;
        assert_eq!(hash, "hash-1");
        Ok(())
    }

    #[test]
    fn claim_release_pending_works() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_pending_schema(&conn)?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO pending_observations
             (session_id, project, tool_name, created_at_epoch, lease_owner, lease_expires_epoch)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params!["s1", "p1", "Edit", now],
        )?;
        conn.execute(
            "INSERT INTO pending_observations
             (session_id, project, tool_name, created_at_epoch, lease_owner, lease_expires_epoch)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params!["s1", "p1", "Bash", now],
        )?;

        let a = claim_pending(&conn, "s1", 1, "owner-a", 60)?;
        assert_eq!(a.len(), 1);
        let b = claim_pending(&conn, "s1", 5, "owner-b", 60)?;
        assert_eq!(b.len(), 1);

        let released = release_pending_claims(&conn, "owner-a")?;
        assert_eq!(released, 1);
        let c = claim_pending(&conn, "s1", 5, "owner-c", 60)?;
        assert_eq!(c.len(), 1);
        Ok(())
    }

    #[test]
    fn cleanup_stale_pending_preserves_active_leases() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_pending_schema(&conn)?;
        let now = chrono::Utc::now().timestamp();
        let old = now - 7200;

        // Stale + unleased: should be deleted.
        conn.execute(
            "INSERT INTO pending_observations
             (session_id, project, tool_name, created_at_epoch, lease_owner, lease_expires_epoch)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params!["s1", "p1", "Edit", old],
        )?;

        // Stale + lease expired: should be deleted.
        conn.execute(
            "INSERT INTO pending_observations
             (session_id, project, tool_name, created_at_epoch, lease_owner, lease_expires_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["s1", "p1", "Bash", old, "owner-expired", now - 10],
        )?;

        // Stale + active lease: should be preserved.
        conn.execute(
            "INSERT INTO pending_observations
             (session_id, project, tool_name, created_at_epoch, lease_owner, lease_expires_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["s1", "p1", "Write", old, "owner-active", now + 600],
        )?;

        // Fresh row: should be preserved.
        conn.execute(
            "INSERT INTO pending_observations
             (session_id, project, tool_name, created_at_epoch, lease_owner, lease_expires_epoch)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params!["s1", "p1", "NotebookEdit", now],
        )?;

        let deleted = cleanup_stale_pending(&conn)?;
        assert_eq!(deleted, 2);

        let remaining: i64 =
            conn.query_row("SELECT COUNT(*) FROM pending_observations", [], |r| {
                r.get(0)
            })?;
        assert_eq!(remaining, 2);

        let active_leased: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE lease_owner = 'owner-active'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(active_leased, 1);
        Ok(())
    }

    #[test]
    fn ai_usage_queries_aggregate() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_usage_schema(&conn)?;
        record_ai_usage(
            &conn,
            Some("p"),
            "flush",
            "cli",
            Some("haiku"),
            100,
            200,
            0.01,
        )?;
        record_ai_usage(
            &conn,
            Some("p"),
            "summarize",
            "cli",
            Some("haiku"),
            50,
            50,
            0.005,
        )?;

        let totals = query_ai_usage_totals(&conn, 7, Some("p"))?;
        assert_eq!(totals.calls, 2);
        assert_eq!(totals.input_tokens, 150);
        assert_eq!(totals.output_tokens, 250);
        assert_eq!(totals.total_tokens, 400);

        let daily = query_ai_usage_daily(&conn, 7, Some("p"))?;
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].calls, 2);

        let events = query_ai_usage_events(&conn, 7, 10, Some("p"))?;
        assert_eq!(events.len(), 2);
        Ok(())
    }

    #[test]
    fn ai_usage_since_filters_old_rows() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_usage_schema(&conn)?;
        let now = chrono::Utc::now().timestamp();
        let old_epoch = now - 3 * 86400;
        let new_epoch = now - 300;
        let old_created = chrono::Utc
            .timestamp_opt(old_epoch, 0)
            .single()
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let new_created = chrono::Utc
            .timestamp_opt(new_epoch, 0)
            .single()
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();

        conn.execute(
            "INSERT INTO ai_usage_events
             (created_at, created_at_epoch, project, operation, executor, model,
              input_tokens, output_tokens, total_tokens, estimated_cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                old_created,
                old_epoch,
                "p",
                "flush",
                "cli",
                "haiku",
                10_i64,
                20_i64,
                30_i64,
                0.001_f64
            ],
        )?;
        conn.execute(
            "INSERT INTO ai_usage_events
             (created_at, created_at_epoch, project, operation, executor, model,
              input_tokens, output_tokens, total_tokens, estimated_cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new_created,
                new_epoch,
                "p",
                "summarize",
                "cli",
                "haiku",
                50_i64,
                60_i64,
                110_i64,
                0.002_f64
            ],
        )?;

        let from_epoch = now - 3600;
        let totals = query_ai_usage_totals_since(&conn, from_epoch, Some("p"))?;
        assert_eq!(totals.calls, 1);
        assert_eq!(totals.total_tokens, 110);

        let daily = query_ai_usage_daily_since(&conn, from_epoch, Some("p"))?;
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].total_tokens, 110);

        let events = query_ai_usage_events_since(&conn, from_epoch, 10, Some("p"))?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "summarize");
        Ok(())
    }
}
