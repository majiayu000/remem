//! Raw archive layer — captures every user/assistant turn regardless of
//! whether summarize/promote choose to keep it.
//!
//! Spec: SPEC-raw-archive-vs-curated-memory-2026-04-22.md

use anyhow::Result;
use rusqlite::{params, Connection};

pub const ROLE_USER: &str = "user";
pub const ROLE_ASSISTANT: &str = "assistant";

pub const SOURCE_TRANSCRIPT: &str = "transcript";
pub const SOURCE_HOOK: &str = "hook";
pub const SOURCE_MANUAL: &str = "manual";

#[derive(Debug, Clone)]
pub struct RawMessage {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub content: String,
    pub source: String,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

/// Exact byte-for-byte hash of the raw message content. Distinct from
/// `memory_promote::slug::content_hash`, which normalizes whitespace/case for
/// semantic dedup of curated memories.
fn exact_content_hash(content: &str) -> String {
    format!("{:016x}", crate::db::deterministic_hash(content.as_bytes()))
}

/// Insert one raw message. UNIQUE(project, role, content_hash) makes this
/// idempotent across repeated Stop-hook drains of the same transcript.
/// Returns the row id of the existing or newly inserted message, or None
/// when the content is empty.
pub fn insert_raw_message(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    source: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
) -> Result<Option<i64>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let hash = exact_content_hash(trimmed);
    let now = chrono::Utc::now().timestamp();

    let inserted = conn.execute(
        "INSERT INTO raw_messages \
         (session_id, project, role, content, content_hash, source, branch, cwd, created_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT(project, role, content_hash) DO NOTHING",
        params![session_id, project, role, trimmed, hash, source, branch, cwd, now],
    )?;

    if inserted > 0 {
        Ok(Some(conn.last_insert_rowid()))
    } else {
        let existing: i64 = conn.query_row(
            "SELECT id FROM raw_messages WHERE project = ?1 AND role = ?2 AND content_hash = ?3",
            params![project, role, hash],
            |row| row.get(0),
        )?;
        Ok(Some(existing))
    }
}

/// Drain a Claude Code transcript JSONL file into raw_messages.
/// Best-effort: any parse error on a single line is skipped.
pub fn drain_transcript(
    conn: &Connection,
    transcript_path: &str,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
) -> Result<usize> {
    let content = match std::fs::read_to_string(transcript_path) {
        Ok(content) => content,
        Err(error) => {
            crate::log::warn(
                "raw-archive",
                &format!("read transcript {} failed: {}", transcript_path, error),
            );
            return Ok(0);
        }
    };

    let mut new_rows = 0usize;
    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let role = match value["type"].as_str() {
            Some("user") => ROLE_USER,
            Some("assistant") => ROLE_ASSISTANT,
            _ => continue,
        };
        let text = extract_message_text(&value);
        if text.trim().is_empty() {
            continue;
        }

        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))
            .unwrap_or(0);
        if let Err(error) = insert_raw_message(
            conn,
            session_id,
            project,
            role,
            &text,
            SOURCE_TRANSCRIPT,
            branch,
            cwd,
        ) {
            crate::log::warn(
                "raw-archive",
                &format!("insert raw message failed: {}", error),
            );
            continue;
        }
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))
            .unwrap_or(before);
        if after > before {
            new_rows += 1;
        }
    }
    Ok(new_rows)
}

fn extract_message_text(value: &serde_json::Value) -> String {
    let content = &value["message"]["content"];
    if let Some(array) = content.as_array() {
        let parts: Vec<String> = array
            .iter()
            .filter_map(|entry| {
                match entry["type"].as_str() {
                    Some("text") => entry["text"].as_str().map(|s| s.to_string()),
                    // user messages can carry plain strings under `content` too
                    _ => None,
                }
            })
            .collect();
        return parts.join("\n");
    }
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    String::new()
}

#[derive(Debug, Clone)]
pub struct RawSearchRequest {
    pub query: String,
    pub project: Option<String>,
    pub role: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub fn search_raw_messages(conn: &Connection, req: &RawSearchRequest) -> Result<Vec<RawMessage>> {
    let limit = req.limit.max(1);
    let offset = req.offset.max(0);
    let query = req.query.trim();
    if query.is_empty() {
        return Ok(vec![]);
    }

    let mut sql = String::from(
        "SELECT r.id, r.session_id, r.project, r.role, r.content, r.source, \
                r.branch, r.cwd, r.created_at_epoch \
         FROM raw_messages r \
         JOIN raw_messages_fts f ON f.rowid = r.id \
         WHERE raw_messages_fts MATCH ?1",
    );
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(fts_query(query))];

    if let Some(project) = req.project.as_deref() {
        sql.push_str(" AND r.project = ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(project.to_string()));
    }
    if let Some(role) = req.role.as_deref() {
        sql.push_str(" AND r.role = ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(role.to_string()));
    }

    sql.push_str(&format!(
        " ORDER BY r.created_at_epoch DESC LIMIT {} OFFSET {}",
        limit, offset
    ));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(crate::db::to_sql_refs(&binds)),
        |row| {
            Ok(RawMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                project: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                source: row.get(5)?,
                branch: row.get(6)?,
                cwd: row.get(7)?,
                created_at_epoch: row.get(8)?,
            })
        },
    )?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn fts_query(query: &str) -> String {
    // Wrap each token in quotes so we use phrase matching (robust against
    // punctuation that trigram tokenizer would otherwise choke on).
    let cleaned: Vec<String> = query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('\"', "\"\"")))
        .collect();
    if cleaned.is_empty() {
        format!("\"{}\"", query.replace('\"', "\"\""))
    } else {
        cleaned.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_is_idempotent_per_project_role_content() {
        let conn = setup_conn();
        let id1 = insert_raw_message(
            &conn,
            "s1",
            "/proj",
            ROLE_USER,
            "hello world",
            SOURCE_HOOK,
            None,
            None,
        )
        .unwrap();
        let id2 = insert_raw_message(
            &conn,
            "s2",
            "/proj",
            ROLE_USER,
            "hello world",
            SOURCE_HOOK,
            None,
            None,
        )
        .unwrap();
        assert_eq!(id1, id2);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn empty_content_is_skipped() {
        let conn = setup_conn();
        let id = insert_raw_message(
            &conn,
            "s1",
            "/proj",
            ROLE_USER,
            "   \n\t  ",
            SOURCE_HOOK,
            None,
            None,
        )
        .unwrap();
        assert!(id.is_none());
    }

    #[test]
    fn fts_finds_inserted_content() {
        let conn = setup_conn();
        insert_raw_message(
            &conn,
            "s1",
            "/proj",
            ROLE_USER,
            "帮我看看 VPS RackNerd 的价格",
            SOURCE_HOOK,
            None,
            None,
        )
        .unwrap();
        let hits = search_raw_messages(
            &conn,
            &RawSearchRequest {
                query: "RackNerd".to_string(),
                project: Some("/proj".to_string()),
                role: None,
                limit: 10,
                offset: 0,
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].content.contains("RackNerd"));
    }
}
