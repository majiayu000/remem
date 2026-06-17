//! Raw archive layer — captures every user/assistant turn regardless of
//! whether summarize/promote choose to keep it.
//!
//! Spec: SPEC-raw-archive-vs-curated-memory-2026-04-22.md

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

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
/// `memory::promote::slug::content_hash`, which normalizes whitespace/case for
/// semantic dedup of curated memories.
fn exact_content_hash(content: &str) -> String {
    crate::db::content_identity_hash(content.as_bytes())
}

fn legacy_exact_content_hash(content: &str) -> String {
    crate::db::legacy_content_identity_hash(content.as_bytes())
}

/// Insert one raw message. UNIQUE(project, session_id, role, content_hash)
/// makes this idempotent across repeated Stop-hook drains of the same
/// transcript while still preserving identical text spoken in different
/// sessions (issue #237).
/// Returns the row id of the existing or newly inserted message, or None
/// when the content is empty.
/// Outcome of a raw-message insert attempt.
///
/// `inserted == false` means a row with the same `(project, session_id, role,
/// content_hash)` already existed and `id` points at the pre-existing row
/// rather than a newly created one.
#[derive(Debug, Clone, Copy)]
pub struct RawInsertOutcome {
    pub id: i64,
    pub inserted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawIngestReport {
    pub inserted: usize,
    pub duplicates: usize,
    pub empty_messages: usize,
    pub skipped_messages: usize,
    pub parse_errors: usize,
    pub insert_errors: usize,
    pub read_error: Option<String>,
}

impl RawIngestReport {
    pub fn has_failures(&self) -> bool {
        self.read_error.is_some() || self.parse_errors > 0 || self.insert_errors > 0
    }

    pub fn failure_kind(&self) -> Option<&'static str> {
        match (
            self.read_error.is_some(),
            self.parse_errors > 0,
            self.insert_errors > 0,
        ) {
            (true, false, false) => Some("read_error"),
            (false, true, false) => Some("parse_errors"),
            (false, false, true) => Some("insert_errors"),
            (true, _, _) | (_, true, true) => Some("mixed_errors"),
            (false, false, false) => None,
        }
    }

    fn failure_message(&self) -> String {
        if let Some(error) = &self.read_error {
            return error.clone();
        }
        format!(
            "parse_errors={} insert_errors={}",
            self.parse_errors, self.insert_errors
        )
    }
}

pub fn insert_raw_message(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    source: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
) -> Result<Option<RawInsertOutcome>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let hash = exact_content_hash(trimmed);
    if let Some(id) = find_matching_legacy_raw_message(conn, session_id, project, role, trimmed)? {
        return Ok(Some(RawInsertOutcome {
            id,
            inserted: false,
        }));
    }
    let now = chrono::Utc::now().timestamp();

    let inserted = conn.execute(
        "INSERT INTO raw_messages \
         (session_id, project, role, content, content_hash, source, branch, cwd, created_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT(project, session_id, role, content_hash) DO NOTHING",
        params![session_id, project, role, trimmed, hash, source, branch, cwd, now],
    )?;

    if inserted > 0 {
        Ok(Some(RawInsertOutcome {
            id: conn.last_insert_rowid(),
            inserted: true,
        }))
    } else {
        let existing: i64 = conn.query_row(
            "SELECT id FROM raw_messages \
             WHERE project = ?1 AND session_id = ?2 AND role = ?3 AND content_hash = ?4",
            params![project, session_id, role, hash],
            |row| row.get(0),
        )?;
        Ok(Some(RawInsertOutcome {
            id: existing,
            inserted: false,
        }))
    }
}

fn find_matching_legacy_raw_message(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
) -> Result<Option<i64>> {
    let legacy_hash = legacy_exact_content_hash(content);
    let Some((id, stored_content)) = conn
        .query_row(
            "SELECT id, content FROM raw_messages
             WHERE project = ?1 AND session_id = ?2 AND role = ?3 AND content_hash = ?4",
            params![project, session_id, role, legacy_hash],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };

    if stored_content == content {
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

/// Drain a Claude Code transcript JSONL file into raw_messages.
pub fn drain_transcript(
    conn: &Connection,
    transcript_path: &str,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
) -> Result<RawIngestReport> {
    let content = match std::fs::read_to_string(transcript_path) {
        Ok(content) => content,
        Err(error) => {
            let report = RawIngestReport {
                read_error: Some(format!(
                    "read transcript {} failed: {}",
                    transcript_path, error
                )),
                ..RawIngestReport::default()
            };
            crate::log::warn(
                "raw-archive",
                report
                    .read_error
                    .as_deref()
                    .unwrap_or("read transcript failed"),
            );
            record_raw_ingest_failure(
                conn,
                session_id,
                project,
                SOURCE_TRANSCRIPT,
                Some(transcript_path),
                &report,
            )?;
            return Ok(report);
        }
    };

    let mut report = RawIngestReport::default();
    with_raw_archive_drain_savepoint(conn, || {
        for line in content.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                report.parse_errors += 1;
                continue;
            };
            let Some(message) = crate::memory::raw_transcript::parse_transcript_message(&value)
            else {
                report.skipped_messages += 1;
                continue;
            };
            if message.text.trim().is_empty() {
                report.empty_messages += 1;
                continue;
            }

            match insert_raw_message(
                conn,
                session_id,
                project,
                message.role,
                &message.text,
                SOURCE_TRANSCRIPT,
                branch,
                cwd,
            ) {
                Ok(Some(outcome)) if outcome.inserted => report.inserted += 1,
                Ok(Some(_)) => report.duplicates += 1,
                Ok(None) => report.empty_messages += 1,
                Err(error) => {
                    report.insert_errors += 1;
                    crate::log::warn(
                        "raw-archive",
                        &format!("insert raw message failed: {}", error),
                    );
                }
            }
        }
        Ok(())
    })?;
    if report.has_failures() {
        record_raw_ingest_failure(
            conn,
            session_id,
            project,
            SOURCE_TRANSCRIPT,
            Some(transcript_path),
            &report,
        )?;
    }
    Ok(report)
}

fn with_raw_archive_drain_savepoint<T>(
    conn: &Connection,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_raw_archive_drain;")
        .context("start raw archive drain savepoint")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_raw_archive_drain;")
                .context("release raw archive drain savepoint")?;
            Ok(value)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_raw_archive_drain;
                 RELEASE SAVEPOINT remem_raw_archive_drain;",
            );
            match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(error).context(format!(
                    "raw archive drain rollback also failed: {rollback_error}"
                )),
            }
        }
    }
}

pub fn record_raw_ingest_failure(
    conn: &Connection,
    session_id: &str,
    project: &str,
    source: &str,
    transcript_path: Option<&str>,
    report: &RawIngestReport,
) -> Result<()> {
    let Some(kind) = report.failure_kind() else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO raw_ingest_failures
         (project, session_id, source, transcript_path, error_kind, error_message,
          inserted, duplicates, parse_errors, insert_errors, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            project,
            session_id,
            source,
            transcript_path,
            kind,
            crate::db::truncate_str(&report.failure_message(), 1000),
            report.inserted as i64,
            report.duplicates as i64,
            report.parse_errors as i64,
            report.insert_errors as i64,
            chrono::Utc::now().timestamp()
        ],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct RawSearchRequest {
    pub query: String,
    pub project: Option<String>,
    pub branch: Option<String>,
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
    if let Some(branch) = req.branch.as_deref() {
        let idx = binds.len() + 1;
        sql.push_str(&format!(" AND (r.branch = ?{idx} OR r.branch IS NULL)"));
        binds.push(Box::new(branch.to_string()));
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
mod tests;
