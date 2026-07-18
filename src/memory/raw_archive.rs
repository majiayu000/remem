use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub const ROLE_USER: &str = "user";
pub const ROLE_ASSISTANT: &str = "assistant";

pub const SOURCE_TRANSCRIPT: &str = "transcript";
pub const SOURCE_HOOK: &str = "hook";
pub const SOURCE_MANUAL: &str = "manual";

pub const SOURCE_ROOT_LOCAL: &str = "local";

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
    /// The last line failed JSON parse while the drain was told to tolerate an
    /// actively-appended tail (issue #722). Not counted as a parse error; the
    /// caller must not advance its ingest cursor so the tail is re-read later.
    pub partial_tail: bool,
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
    insert_raw_message_from_root(
        conn,
        session_id,
        project,
        role,
        content,
        source,
        branch,
        cwd,
        SOURCE_ROOT_LOCAL,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_raw_message_from_root(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    source: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
    source_root: &str,
) -> Result<Option<RawInsertOutcome>> {
    insert_raw_message_from_root_at(
        conn,
        session_id,
        project,
        role,
        content,
        source,
        branch,
        cwd,
        source_root,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_raw_message_from_root_at(
    conn: &Connection,
    session_id: &str,
    project: &str,
    role: &str,
    content: &str,
    source: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
    source_root: &str,
    created_at_epoch: Option<i64>,
) -> Result<Option<RawInsertOutcome>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let hash = exact_content_hash(trimmed);
    if let Some(id) =
        find_matching_legacy_raw_message(conn, session_id, project, role, trimmed, source_root)?
    {
        return Ok(Some(RawInsertOutcome {
            id,
            inserted: false,
        }));
    }
    let inserted_at = created_at_epoch.unwrap_or_else(|| chrono::Utc::now().timestamp());

    let event_time_source = if source == SOURCE_TRANSCRIPT {
        if created_at_epoch.is_some() {
            "transcript_event"
        } else {
            "ingest_fallback"
        }
    } else {
        "legacy_unknown"
    };
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO raw_messages \
         (session_id, project, role, content, content_hash, source, branch, cwd, \
          created_at_epoch, source_root, event_time_source) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            session_id,
            project,
            role,
            trimmed,
            hash,
            source,
            branch,
            cwd,
            inserted_at,
            source_root,
            event_time_source
        ],
    )?;

    if inserted > 0 {
        Ok(Some(RawInsertOutcome {
            id: conn.last_insert_rowid(),
            inserted: true,
        }))
    } else {
        let existing: i64 = conn.query_row(
            "SELECT id FROM raw_messages \
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 \
               AND role = ?4 AND content_hash = ?5
               AND transcript_identity_id IS NULL",
            params![source_root, project, session_id, role, hash],
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
    source_root: &str,
) -> Result<Option<i64>> {
    let legacy_hash = legacy_exact_content_hash(content);
    let Some((id, stored_content)) = conn
        .query_row(
            "SELECT id, content FROM raw_messages
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 \
               AND role = ?4 AND content_hash = ?5
               AND transcript_identity_id IS NULL",
            params![source_root, project, session_id, role, legacy_hash],
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

/// Options for a transcript drain beyond the hook-path defaults.
#[derive(Debug, Clone)]
pub struct TranscriptDrainOptions<'a> {
    /// Label of the scan root the transcript came from (`local` for the hook
    /// path and default `ingest-sessions` roots).
    pub source_root: &'a str,
    /// Treat a JSON parse failure on the final line as an actively-appended
    /// partial tail instead of a parse error (issue #722). The caller decides
    /// this from the file mtime; see `RawIngestReport::partial_tail`.
    pub tolerate_partial_tail: bool,
    /// Stable local transcript identity. When present, line ordinals preserve
    /// repeated identical turns while making Stop/batch replay idempotent.
    pub transcript_identity_id: Option<i64>,
}

impl Default for TranscriptDrainOptions<'_> {
    fn default() -> Self {
        Self {
            source_root: SOURCE_ROOT_LOCAL,
            tolerate_partial_tail: false,
            transcript_identity_id: None,
        }
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
    drain_transcript_with_options(
        conn,
        transcript_path,
        session_id,
        project,
        branch,
        cwd,
        &TranscriptDrainOptions::default(),
    )
}

pub fn raw_ingest_status(report: &RawIngestReport) -> &'static str {
    if report.read_error.is_some() {
        "read_failed"
    } else if report.parse_errors > 0 || report.insert_errors > 0 {
        "partial"
    } else if report.inserted == 0 && report.duplicates > 0 {
        "duplicate_only"
    } else {
        "ok"
    }
}

/// Drain a transcript with an explicit source root and partial-tail policy.
#[allow(clippy::too_many_arguments)]
pub fn drain_transcript_with_options(
    conn: &Connection,
    transcript_path: &str,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
    options: &TranscriptDrainOptions<'_>,
) -> Result<RawIngestReport> {
    drain_transcript_with_capture_limit(
        conn,
        transcript_path,
        session_id,
        project,
        branch,
        cwd,
        options,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_transcript_with_capture_limit(
    conn: &Connection,
    transcript_path: &str,
    session_id: &str,
    project: &str,
    branch: Option<&str>,
    cwd: Option<&str>,
    options: &TranscriptDrainOptions<'_>,
    byte_limit: Option<u64>,
) -> Result<RawIngestReport> {
    let mut report = RawIngestReport::default();
    let mut record_ordinal = 0_i64;
    let stream_result = with_raw_archive_drain_savepoint(conn, || {
        crate::memory::raw_transcript::stream_transcript_lines(
            transcript_path,
            byte_limit,
            |line, is_final| {
                let ordinal = record_ordinal;
                record_ordinal += 1;
                use crate::memory::raw_transcript::TranscriptRecordClass;
                let message =
                    match crate::memory::raw_transcript::classify_transcript_line(line, None) {
                        TranscriptRecordClass::Conversation(message)
                        | TranscriptRecordClass::MetaUser(message)
                        | TranscriptRecordClass::XmlControlUser(message)
                        | TranscriptRecordClass::MissingEventTime(message) => message,
                        TranscriptRecordClass::MalformedRecord => {
                            if options.tolerate_partial_tail && is_final {
                                report.partial_tail = true;
                            } else {
                                report.parse_errors += 1;
                            }
                            return;
                        }
                        TranscriptRecordClass::EmptyText => {
                            report.empty_messages += 1;
                            return;
                        }
                        TranscriptRecordClass::UnsupportedRecord
                        | TranscriptRecordClass::OutsideWindow => {
                            report.skipped_messages += 1;
                            return;
                        }
                    };
                let insert_result = if let Some(identity_id) = options.transcript_identity_id {
                    crate::memory::raw_occurrence::insert_transcript_occurrence(
                        conn,
                        session_id,
                        project,
                        message.role,
                        &message.text,
                        branch,
                        cwd,
                        options.source_root,
                        message.created_at_epoch,
                        identity_id,
                        ordinal,
                    )
                } else {
                    insert_raw_message_from_root_at(
                        conn,
                        session_id,
                        project,
                        message.role,
                        &message.text,
                        SOURCE_TRANSCRIPT,
                        branch,
                        cwd,
                        options.source_root,
                        message.created_at_epoch,
                    )
                };
                match insert_result {
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
            },
        )
        .map_err(anyhow::Error::from)?;
        if report.parse_errors > 0 || report.insert_errors > 0 {
            anyhow::bail!("raw archive drain validation failed");
        }
        Ok(())
    })?;
    if let Err(error) = stream_result {
        report.inserted = 0;
        report.duplicates = 0;
        if report.parse_errors == 0 && report.insert_errors == 0 {
            report.read_error = Some(format!("read transcript {transcript_path} failed: {error}"));
            crate::log::warn(
                "raw-archive",
                report
                    .read_error
                    .as_deref()
                    .unwrap_or("read transcript failed"),
            );
        }
    }
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
) -> Result<std::result::Result<T, anyhow::Error>> {
    conn.execute_batch("SAVEPOINT remem_raw_archive_drain;")
        .context("start raw archive drain savepoint")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_raw_archive_drain;")
                .context("release raw archive drain savepoint")?;
            Ok(Ok(value))
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_raw_archive_drain;
                 RELEASE SAVEPOINT remem_raw_archive_drain;",
            );
            match rollback {
                Ok(()) => Ok(Err(error)),
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
    /// Inclusive lower bound on `created_at_epoch`. None keeps the
    /// pre-window behavior (issue #723).
    pub since_epoch: Option<i64>,
    /// Inclusive upper bound on `created_at_epoch`. None keeps the
    /// pre-window behavior (issue #723).
    pub until_epoch: Option<i64>,
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
    if let Some(since) = req.since_epoch {
        sql.push_str(" AND r.created_at_epoch >= ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(since));
    }
    if let Some(until) = req.until_epoch {
        sql.push_str(" AND r.created_at_epoch <= ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(until));
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

/// Query parameters for the window session listing (issue #723).
#[derive(Debug, Clone, Default)]
pub struct RawSessionQuery {
    /// Inclusive lower bound on `created_at_epoch`.
    pub since_epoch: Option<i64>,
    /// Inclusive upper bound on `created_at_epoch`.
    pub until_epoch: Option<i64>,
    /// Restrict to one project path.
    pub project: Option<String>,
    /// Sample up to this many role=user message texts per session, ascending
    /// by epoch. 0 disables sampling.
    pub sample_user_messages: i64,
}

/// Truncation bound for sampled user message texts.
const SESSION_SAMPLE_PREVIEW_CHARS: usize = 200;

/// One session seen inside the query window. Serialized shape is the shared
/// CLI/MCP JSON contract (product invariant 10).
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct RawSessionSummary {
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    /// Min/max `created_at_epoch` among the session's messages in the window.
    pub first_epoch: i64,
    pub last_epoch: i64,
    pub message_count: i64,
    pub user_message_count: i64,
    pub assistant_message_count: i64,
    /// First N role=user message texts (truncated), ascending by epoch.
    pub user_message_samples: Vec<String>,
}

/// List sessions with messages inside the window, grouped by
/// `(source_root, project, session_id)` and ordered by first message epoch.
pub fn list_sessions(conn: &Connection, query: &RawSessionQuery) -> Result<Vec<RawSessionSummary>> {
    let mut sql = String::from(
        "SELECT source_root, project, session_id, \
                MIN(created_at_epoch), MAX(created_at_epoch), COUNT(*), \
                SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END), \
                SUM(CASE WHEN role = 'assistant' THEN 1 ELSE 0 END) \
         FROM raw_messages WHERE 1=1",
    );
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    push_session_filters(&mut sql, &mut binds, query);
    sql.push_str(" GROUP BY source_root, project, session_id ORDER BY MIN(created_at_epoch) ASC");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(crate::db::to_sql_refs(&binds)),
        |row| {
            Ok(RawSessionSummary {
                source_root: row.get(0)?,
                project: row.get(1)?,
                session_id: row.get(2)?,
                first_epoch: row.get(3)?,
                last_epoch: row.get(4)?,
                message_count: row.get(5)?,
                user_message_count: row.get(6)?,
                assistant_message_count: row.get(7)?,
                user_message_samples: Vec::new(),
            })
        },
    )?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    if query.sample_user_messages > 0 {
        for session in &mut sessions {
            session.user_message_samples = sample_user_messages(conn, query, session)?;
        }
    }
    Ok(sessions)
}

fn push_session_filters(
    sql: &mut String,
    binds: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    query: &RawSessionQuery,
) {
    if let Some(project) = query.project.as_deref() {
        sql.push_str(" AND project = ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(project.to_string()));
    }
    if let Some(since) = query.since_epoch {
        sql.push_str(" AND created_at_epoch >= ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(since));
    }
    if let Some(until) = query.until_epoch {
        sql.push_str(" AND created_at_epoch <= ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(until));
    }
}

fn sample_user_messages(
    conn: &Connection,
    query: &RawSessionQuery,
    session: &RawSessionSummary,
) -> Result<Vec<String>> {
    let mut sql = String::from(
        "SELECT content FROM raw_messages \
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 AND role = ?4",
    );
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(session.source_root.clone()),
        Box::new(session.project.clone()),
        Box::new(session.session_id.clone()),
        Box::new(ROLE_USER.to_string()),
    ];
    if let Some(since) = query.since_epoch {
        sql.push_str(" AND created_at_epoch >= ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(since));
    }
    if let Some(until) = query.until_epoch {
        sql.push_str(" AND created_at_epoch <= ?");
        sql.push_str(&(binds.len() + 1).to_string());
        binds.push(Box::new(until));
    }
    sql.push_str(&format!(
        " ORDER BY created_at_epoch ASC, id ASC LIMIT {}",
        query.sample_user_messages.max(0)
    ));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(crate::db::to_sql_refs(&binds)),
        |row| row.get::<_, String>(0),
    )?;
    let mut samples = Vec::new();
    for row in rows {
        let content = row?;
        samples.push(content.chars().take(SESSION_SAMPLE_PREVIEW_CHARS).collect());
    }
    Ok(samples)
}

/// Shared CLI/MCP JSON envelope for the window session listing so both
/// surfaces emit identical fields (product invariant 10).
#[derive(Debug, Clone, serde::Serialize)]
pub struct RawSessionsJson {
    pub since_epoch: Option<i64>,
    pub until_epoch: Option<i64>,
    pub project: Option<String>,
    pub sample: i64,
    pub count: usize,
    pub sessions: Vec<RawSessionSummary>,
}

pub fn build_sessions_json(
    query: &RawSessionQuery,
    sessions: Vec<RawSessionSummary>,
) -> RawSessionsJson {
    RawSessionsJson {
        since_epoch: query.since_epoch,
        until_epoch: query.until_epoch,
        project: query.project.clone(),
        sample: query.sample_user_messages,
        count: sessions.len(),
        sessions,
    }
}

/// Parse a time bound given as Unix epoch seconds, an ISO8601 datetime, or a
/// plain `YYYY-MM-DD` date interpreted as UTC midnight.
///
/// This public compatibility entry point retains its original date semantics.
/// Query surfaces that need an inclusive date-only upper bound use the
/// transport-neutral upper-bound parser instead.
pub fn parse_time_bound(value: &str) -> Result<i64> {
    super::raw_query::parse_time_lower_bound(value)
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
