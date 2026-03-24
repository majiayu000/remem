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
    /// Git branch name associated with this memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Scope: "project" (default, only visible in this project) or "global" (visible everywhere).
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
                return Ok(id);
            }
        }
    }

    conn.execute(
        "INSERT INTO memories \
         (session_id, project, topic_key, title, content, memory_type, files, \
          created_at_epoch, updated_at_epoch, status, branch, scope) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'active', ?9, ?10)",
        params![
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            now,
            branch,
            scope
        ],
    )?;
    Ok(conn.last_insert_rowid())
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
        "SELECT id, session_id, project, topic_key, title, content, memory_type, files, \
         created_at_epoch, updated_at_epoch, status, branch, scope \
         FROM memories WHERE {} ORDER BY updated_at_epoch DESC",
        conditions.join(" AND ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_memory_row)?;
    crate::db_query::collect_rows(rows)
}

/// Push project suffix-match filter into SQL conditions.
/// "harness" matches exact "harness" OR ends with "/harness".
/// Returns the next parameter index.
fn push_project_suffix_filter(
    column: &str,
    project: Option<&str>,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> usize {
    if let Some(p) = project {
        conditions.push(format!("({column} = ?{idx} OR {column} LIKE ?{})", idx + 1));
        params.push(Box::new(p.to_string()));
        params.push(Box::new(format!("%/{p}")));
        idx += 2;
    }
    idx
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

    idx = push_project_suffix_filter("m.project", project, idx, &mut conditions, &mut param_values);
    if let Some(t) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
        idx += 1;
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content, \
         m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch, m.status, m.branch, m.scope \
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

    idx = push_project_suffix_filter("m.project", project, idx, &mut conditions, &mut param_values);
    if let Some(t) = memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
        idx += 1;
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content, \
         m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch, m.status, m.branch, m.scope \
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

// --- Auto-promote from session summaries ---

/// Minimum content length to be worth promoting.
const MIN_DECISION_LEN: usize = 30;
const MIN_LEARNED_LEN: usize = 30;
const MIN_PREFERENCE_LEN: usize = 10;

/// Generate a stable topic_key from text for UPSERT dedup.
pub fn slugify_for_topic(text: &str, max_len: usize) -> String {
    slugify(text, max_len)
}

fn slugify(text: &str, max_len: usize) -> String {
    let slug: String = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else if c == '-' || c == '_' || c == ' ' {
                '-'
            } else if !c.is_ascii() {
                // Keep CJK and other unicode chars for meaningful keys
                c
            } else {
                '-'
            }
        })
        .collect();
    // Collapse multiple dashes and trim
    let mut result = String::with_capacity(slug.len());
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash && !result.is_empty() {
                result.push('-');
            }
            last_dash = true;
        } else {
            result.push(c);
            last_dash = false;
        }
    }
    let trimmed = result.trim_end_matches('-');
    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        // Truncate at char boundary
        trimmed.chars().take(max_len).collect()
    }
}

/// Split a multi-line text block into individual items.
/// Recognizes bullet points (•, -, *), numbered lists, and line breaks.
fn split_into_items(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Detect list item start: bullet points, numbered, or semicolon-separated
        let is_new_item = trimmed.starts_with("• ")
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("· ")
            || trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
                && trimmed.contains(". ");

        if is_new_item {
            if !current.trim().is_empty() {
                items.push(current.trim().to_string());
            }
            // Strip the bullet/number prefix
            let content = trimmed
                .trim_start_matches(|c: char| c == '•' || c == '-' || c == '*' || c == '·')
                .trim_start();
            let content = if content
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                // Strip "1. " prefix
                content
                    .find(". ")
                    .map(|pos| &content[pos + 2..])
                    .unwrap_or(content)
            } else {
                content
            };
            current = content.to_string();
        } else {
            // Continuation line
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(trimmed);
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }

    // If no list items detected, try splitting by semicolons
    if items.len() <= 1 {
        let original = text.trim();
        let semi_split: Vec<String> = original
            .split('；')
            .flat_map(|s| s.split(';'))
            .map(|s| s.trim().to_string())
            .filter(|s| s.len() >= MIN_DECISION_LEN)
            .collect();
        if semi_split.len() > 1 {
            return semi_split;
        }
    }

    items
}

/// Auto-promote session summary fields to memories.
/// Called after successful finalize_summarize(). Zero LLM cost.
/// Splits multi-item decisions/learned into individual memories for better search precision.
/// Returns number of memories created/updated.
pub fn promote_summary_to_memories(
    conn: &Connection,
    session_id: &str,
    project: &str,
    request: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    preferences: Option<&str>,
) -> Result<usize> {
    let request_text = request.unwrap_or("").trim();
    let mut count = 0;

    // Promote decisions → memory_type="decision" (split into individual items)
    if let Some(text) = decisions {
        let text = text.trim();
        if text.len() >= MIN_DECISION_LEN {
            let items = split_into_items(text);
            if items.len() > 1 {
                // Multiple decisions: create one memory per decision
                for (i, item) in items.iter().enumerate() {
                    if item.len() < MIN_DECISION_LEN {
                        continue;
                    }
                    let title = if request_text.is_empty() {
                        format!("Decision: {}", &item[..item.len().min(70)])
                    } else {
                        let preview = &request_text[..request_text.len().min(60)];
                        format!("{} — decision {}", preview, i + 1)
                    };
                    let topic_key =
                        format!("auto-decision-{}-{}", slugify(request_text, 40), i + 1);
                    insert_memory(
                        conn,
                        Some(session_id),
                        project,
                        Some(&topic_key),
                        &title,
                        item,
                        "decision",
                        None,
                    )?;
                    count += 1;
                }
            } else {
                // Single decision: keep as-is
                let title = if request_text.is_empty() {
                    "Session decisions".to_string()
                } else {
                    let preview = &request_text[..request_text.len().min(80)];
                    format!("{} — decisions", preview)
                };
                let content = if request_text.is_empty() {
                    text.to_string()
                } else {
                    format!("**Request**: {}\n\n**Decisions**: {}", request_text, text)
                };
                let topic_key = format!("auto-decision-{}", slugify(request_text, 50));
                insert_memory(
                    conn,
                    Some(session_id),
                    project,
                    Some(&topic_key),
                    &title,
                    &content,
                    "decision",
                    None,
                )?;
                count += 1;
            }
        }
    }

    // Promote learned → memory_type="discovery" (split into individual items)
    if let Some(text) = learned {
        let text = text.trim();
        if text.len() >= MIN_LEARNED_LEN {
            let items = split_into_items(text);
            if items.len() > 1 {
                for (i, item) in items.iter().enumerate() {
                    if item.len() < MIN_LEARNED_LEN {
                        continue;
                    }
                    let title = if request_text.is_empty() {
                        format!("Discovery: {}", &item[..item.len().min(70)])
                    } else {
                        let preview = &request_text[..request_text.len().min(60)];
                        format!("{} — discovery {}", preview, i + 1)
                    };
                    let topic_key =
                        format!("auto-discovery-{}-{}", slugify(request_text, 40), i + 1);
                    insert_memory(
                        conn,
                        Some(session_id),
                        project,
                        Some(&topic_key),
                        &title,
                        item,
                        "discovery",
                        None,
                    )?;
                    count += 1;
                }
            } else {
                let title = if request_text.is_empty() {
                    "Session insights".to_string()
                } else {
                    let preview = &request_text[..request_text.len().min(80)];
                    format!("{} — learned", preview)
                };
                let content = if request_text.is_empty() {
                    text.to_string()
                } else {
                    format!("**Request**: {}\n\n**Learned**: {}", request_text, text)
                };
                let topic_key = format!("auto-discovery-{}", slugify(request_text, 50));
                insert_memory(
                    conn,
                    Some(session_id),
                    project,
                    Some(&topic_key),
                    &title,
                    &content,
                    "discovery",
                    None,
                )?;
                count += 1;
            }
        }
    }

    // Promote preferences → memory_type="preference", scope="global"
    // Preferences are user-level knowledge that applies across all projects
    if let Some(text) = preferences {
        let text = text.trim();
        if text.len() >= MIN_PREFERENCE_LEN {
            let title = format!("Preference: {}", &text[..text.len().min(60)]);
            let topic_key = format!("auto-preference-{}", slugify(text, 50));
            insert_memory_full(
                conn,
                Some(session_id),
                project,
                Some(&topic_key),
                &title,
                text,
                "preference",
                None,
                None,
                "global",
            )?;
            count += 1;
        }
    }

    if count > 0 {
        crate::log::info(
            "promote",
            &format!(
                "promoted {} memories from summary project={}",
                count, project
            ),
        );
    }

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

/// Column list for all memory SELECT queries.
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
    fn test_promote_summary_decisions() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Fix FTS5 search bug"),
            Some("Switched from unicode61 to trigram tokenizer for better CJK support"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(count, 1);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].memory_type, "decision");
        assert!(memories[0].title.contains("decisions"));
        assert!(memories[0].text.contains("trigram"));
        let topic = memories[0].topic_key.as_deref().unwrap_or_default();
        assert!(topic.starts_with("auto-decision-"));
    }

    #[test]
    fn test_promote_summary_all_fields() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Add workstream support"),
            Some("Used priority-based job queue for workstream processing"),
            Some("WorkStream state transitions need careful ordering to avoid race conditions"),
            Some("User prefers Chinese comments in code"),
        )
        .unwrap();
        assert_eq!(count, 3);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 3);

        let types: Vec<&str> = memories.iter().map(|m| m.memory_type.as_str()).collect();
        assert!(types.contains(&"decision"));
        assert!(types.contains(&"discovery"));
        assert!(types.contains(&"preference"));
    }

    #[test]
    fn test_promote_skips_short_content() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Quick fix"),
            Some("minor"), // < 30 chars, should be skipped
            Some("short"), // < 30 chars, should be skipped
            None,
        )
        .unwrap();
        assert_eq!(count, 0);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert!(memories.is_empty());
    }

    #[test]
    fn test_promote_upsert_same_topic() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        // First session
        promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Fix FTS5 search"),
            Some("Initial approach: use unicode61 tokenizer for word boundary detection"),
            None,
            None,
        )
        .unwrap();

        // Second session with same request → should UPSERT
        promote_summary_to_memories(
            &conn,
            "session-2",
            "test/proj",
            Some("Fix FTS5 search"),
            Some("Switched to trigram tokenizer — unicode61 fails on CJK characters"),
            None,
            None,
        )
        .unwrap();

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        // Same topic_key → updated, not duplicated
        assert_eq!(memories.len(), 1);
        assert!(memories[0].text.contains("trigram"));
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

    #[test]
    fn test_split_into_items_bullets() {
        let text = "• Use RwLock for concurrent reads\n• Switch to trigram tokenizer\n• Set compression threshold=100";
        let items = split_into_items(text);
        assert_eq!(items.len(), 3);
        assert!(items[0].contains("RwLock"));
        assert!(items[1].contains("trigram"));
        assert!(items[2].contains("compression"));
    }

    #[test]
    fn test_split_into_items_dashes() {
        let text = "- First decision about architecture\n- Second decision about testing\n- Third one";
        let items = split_into_items(text);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_split_into_items_single_line() {
        let text = "Switched from unicode61 to trigram tokenizer for better CJK support";
        let items = split_into_items(text);
        assert_eq!(items.len(), 1);
        assert!(items[0].contains("trigram"));
    }

    #[test]
    fn test_split_into_items_semicolons() {
        let text = "Use RwLock for concurrent reads; Switch to trigram tokenizer for CJK; Set compression threshold to 100 observations";
        let items = split_into_items(text);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_promote_multi_decisions() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let decisions = "• Use RwLock instead of Mutex for concurrent read support\n\
                         • Switch to trigram tokenizer for CJK text search\n\
                         • Set compression threshold to 100 observations";
        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Optimize search and concurrency"),
            Some(decisions),
            None,
            None,
        )
        .unwrap();
        assert_eq!(count, 3);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 3);
        for m in &memories {
            assert_eq!(m.memory_type, "decision");
        }
    }

    #[test]
    fn test_promote_multi_learned() {
        let conn = Connection::open_in_memory().unwrap();
        setup_memory_schema(&conn);

        let learned = "- FTS5 trigram tokenizer handles CJK without word boundaries\n\
                       - WAL mode allows concurrent reads with single writer";
        let count = promote_summary_to_memories(
            &conn,
            "session-1",
            "test/proj",
            Some("Research storage"),
            None,
            Some(learned),
            None,
        )
        .unwrap();
        assert_eq!(count, 2);

        let memories = get_recent_memories(&conn, "test/proj", 10).unwrap();
        assert_eq!(memories.len(), 2);
        for m in &memories {
            assert_eq!(m.memory_type, "discovery");
        }
    }
}
