use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

const DIRECT_CONTENT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionTaskKind {
    SessionRollup,
    ObservationExtract,
    MemoryCandidate,
    GraphCandidate,
    RuleCandidate,
    IndexUpdate,
}

impl ExtractionTaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionRollup => "session_rollup",
            Self::ObservationExtract => "observation_extract",
            Self::MemoryCandidate => "memory_candidate",
            Self::GraphCandidate => "graph_candidate",
            Self::RuleCandidate => "rule_candidate",
            Self::IndexUpdate => "index_update",
        }
    }

    pub fn from_db(raw: &str) -> Result<Self> {
        match raw {
            "session_rollup" => Ok(Self::SessionRollup),
            "observation_extract" => Ok(Self::ObservationExtract),
            "memory_candidate" => Ok(Self::MemoryCandidate),
            "graph_candidate" => Ok(Self::GraphCandidate),
            "rule_candidate" => Ok(Self::RuleCandidate),
            "index_update" => Ok(Self::IndexUpdate),
            _ => bail!("unknown extraction task kind: {raw}"),
        }
    }

    pub(crate) fn priority(self) -> i64 {
        match self {
            Self::SessionRollup => 10,
            Self::ObservationExtract => 20,
            Self::MemoryCandidate => 40,
            Self::GraphCandidate => 50,
            Self::RuleCandidate => 60,
            Self::IndexUpdate => 80,
        }
    }
}

pub struct CaptureEventInput<'a> {
    pub host: &'a str,
    pub session_id: &'a str,
    pub project: &'a str,
    pub cwd: Option<&'a str>,
    pub event_type: &'a str,
    pub role: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub content: &'a str,
    pub task_kind: Option<ExtractionTaskKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureEventOutcome {
    pub event_row_id: i64,
    pub event_id: String,
    pub extraction_task_id: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct IdentityIds {
    host_id: i64,
    workspace_id: i64,
    project_id: i64,
    session_row_id: i64,
}

pub fn record_captured_event(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
) -> Result<CaptureEventOutcome> {
    record_captured_event_with_id(conn, input, None)
}

pub fn record_captured_event_with_id(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    event_id_override: Option<&str>,
) -> Result<CaptureEventOutcome> {
    let now = chrono::Utc::now().timestamp();
    record_captured_event_inner(conn, input, event_id_override, now, now, None)
}

pub fn record_captured_event_with_id_and_reference_time(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    event_id_override: Option<&str>,
    reference_time_epoch: Option<i64>,
) -> Result<CaptureEventOutcome> {
    let now = chrono::Utc::now().timestamp();
    let created_at_epoch = reference_time_epoch.unwrap_or(now);
    record_captured_event_inner(
        conn,
        input,
        event_id_override,
        created_at_epoch,
        now,
        reference_time_epoch,
    )
}

pub fn record_captured_event_with_id_and_created_at(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    event_id_override: Option<&str>,
    created_at_epoch: i64,
) -> Result<CaptureEventOutcome> {
    let now = chrono::Utc::now().timestamp();
    record_captured_event_inner(
        conn,
        input,
        event_id_override,
        created_at_epoch,
        now,
        Some(created_at_epoch),
    )
}

fn record_captured_event_inner(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    event_id_override: Option<&str>,
    created_at_epoch: i64,
    now: i64,
    reference_time_epoch: Option<i64>,
) -> Result<CaptureEventOutcome> {
    let inserted_at = now;
    let sanitized_content = redact_capture_content(input.content);
    let content_hash = exact_hash(&sanitized_content);
    let event_id = event_id_override
        .map(ToString::to_string)
        .unwrap_or_else(|| synthesize_event_id(input.event_type, &content_hash));
    let identity = upsert_identity(conn, input, now)?;
    let (content_text, content_blob_id, retention_class) =
        store_content(conn, &sanitized_content, &content_hash, now)?;
    let token_estimate = estimate_tokens(&sanitized_content);

    conn.execute(
        "INSERT INTO captured_events
         (host_id, workspace_id, project_id, session_row_id, session_id, turn_id,
          event_id, event_type, role, tool_name, content_text, content_blob_id,
          content_hash, token_estimate, retention_class, created_at_epoch, inserted_at_epoch,
          reference_time_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
         ON CONFLICT(host_id, session_id, event_id) DO UPDATE SET
             inserted_at_epoch = excluded.inserted_at_epoch,
             reference_time_epoch = COALESCE(excluded.reference_time_epoch, captured_events.reference_time_epoch)",
        params![
            identity.host_id,
            identity.workspace_id,
            identity.project_id,
            identity.session_row_id,
            input.session_id,
            event_id,
            input.event_type,
            input.role,
            input.tool_name,
            content_text,
            content_blob_id,
            content_hash,
            token_estimate,
            retention_class,
            created_at_epoch,
            inserted_at,
            reference_time_epoch
        ],
    )?;

    let event_row_id = conn.query_row(
        "SELECT id FROM captured_events WHERE host_id = ?1 AND session_id = ?2 AND event_id = ?3",
        params![identity.host_id, input.session_id, event_id],
        |row| row.get(0),
    )?;

    let extraction_task_id = if let Some(kind) = input.task_kind {
        Some(coalesce_extraction_task(
            conn,
            identity,
            kind,
            event_row_id,
            now,
        )?)
    } else {
        None
    };

    Ok(CaptureEventOutcome {
        event_row_id,
        event_id,
        extraction_task_id,
    })
}

fn upsert_identity(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    now: i64,
) -> Result<IdentityIds> {
    let host_id = upsert_host(conn, normalize_host(input.host)?, now)?;
    let root_path = input.project.to_string();
    let git_branch = input.cwd.and_then(crate::db::detect_git_branch);
    let workspace_id = upsert_workspace(conn, &root_path, git_branch.as_deref(), now)?;
    let project_id = upsert_project(conn, workspace_id, input.project, now)?;
    let session_row_id = upsert_session_row(
        conn,
        host_id,
        workspace_id,
        project_id,
        input.session_id,
        now,
    )?;
    Ok(IdentityIds {
        host_id,
        workspace_id,
        project_id,
        session_row_id,
    })
}

fn normalize_host(host: &str) -> Result<&str> {
    match host {
        "claude-code" | "codex-cli" => Ok(host),
        other => bail!("invalid capture host '{other}'; expected claude-code or codex-cli"),
    }
}

fn upsert_host(conn: &Connection, name: &str, now: i64) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO hosts(name, enabled, created_at_epoch) VALUES (?1, 1, ?2)",
        params![name, now],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM hosts WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?)
}

fn upsert_workspace(
    conn: &Connection,
    root_path: &str,
    git_branch: Option<&str>,
    now: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
         VALUES (?1, NULL, ?2, ?3, ?3)
         ON CONFLICT(root_path) DO UPDATE SET
             git_branch = COALESCE(excluded.git_branch, workspaces.git_branch),
             updated_at_epoch = excluded.updated_at_epoch",
        params![root_path, git_branch, now],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM workspaces WHERE root_path = ?1",
        params![root_path],
        |row| row.get(0),
    )?)
}

fn upsert_project(
    conn: &Connection,
    workspace_id: i64,
    project_path: &str,
    now: i64,
) -> Result<i64> {
    let project_key = project_path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(project_path);
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(workspace_id, project_path) DO UPDATE SET
             project_key = excluded.project_key,
             updated_at_epoch = excluded.updated_at_epoch",
        params![workspace_id, project_path, project_key, now],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM projects WHERE workspace_id = ?1 AND project_path = ?2",
        params![workspace_id, project_path],
        |row| row.get(0),
    )?)
}

fn upsert_session_row(
    conn: &Connection,
    host_id: i64,
    workspace_id: i64,
    project_id: i64,
    session_id: &str,
    now: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch, last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'active')
         ON CONFLICT(host_id, project_id, session_id) DO UPDATE SET
             last_seen_at_epoch = excluded.last_seen_at_epoch,
             status = 'active'",
        params![host_id, workspace_id, project_id, session_id, now],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM sessions WHERE host_id = ?1 AND project_id = ?2 AND session_id = ?3",
        params![host_id, project_id, session_id],
        |row| row.get(0),
    )?)
}

fn store_content(
    conn: &Connection,
    content: &str,
    content_hash: &str,
    now: i64,
) -> Result<(String, Option<i64>, &'static str)> {
    if content.len() <= DIRECT_CONTENT_BYTES {
        return Ok((content.to_string(), None, "raw_keep"));
    }

    let bytes = content.as_bytes();
    if let Some(blob_id) = matching_legacy_blob_id(conn, content)? {
        return Ok((
            compact_preview(content, DIRECT_CONTENT_BYTES),
            Some(blob_id),
            "raw_compact",
        ));
    }

    conn.execute(
        "INSERT INTO event_blobs(content_hash, content_encoding, content_bytes, original_bytes, stored_bytes, created_at_epoch)
         VALUES (?1, 'plain', ?2, ?3, ?3, ?4)
         ON CONFLICT(content_hash) DO NOTHING",
        params![content_hash, bytes, bytes.len() as i64, now],
    )?;
    let blob_id: i64 = conn
        .query_row(
            "SELECT id FROM event_blobs WHERE content_hash = ?1",
            params![content_hash],
            |row| row.get(0),
        )
        .optional()?
        .expect("event blob row should exist after insert");
    Ok((
        compact_preview(content, DIRECT_CONTENT_BYTES),
        Some(blob_id),
        "raw_compact",
    ))
}

fn matching_legacy_blob_id(conn: &Connection, content: &str) -> Result<Option<i64>> {
    let legacy_hash = legacy_exact_hash(content);
    let Some((id, encoding, bytes)) = conn
        .query_row(
            "SELECT id, content_encoding, content_bytes FROM event_blobs WHERE content_hash = ?1",
            params![legacy_hash],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    if encoding == "plain" && bytes == content.as_bytes() {
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

fn coalesce_extraction_task(
    conn: &Connection,
    identity: IdentityIds,
    kind: ExtractionTaskKind,
    event_row_id: i64,
    now: i64,
) -> Result<i64> {
    let idempotency_key = format!(
        "{}:{}:{}:{}",
        identity.host_id,
        identity.project_id,
        identity.session_row_id,
        kind.as_str()
    );
    conn.execute(
        "INSERT INTO extraction_tasks
         (task_kind, host_id, workspace_id, project_id, session_row_id, priority, status,
          idempotency_key, cursor_event_id, high_watermark_event_id, attempts,
          next_retry_epoch, lease_owner, lease_expires_epoch, last_error, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, NULL, ?8, 0, NULL, NULL, NULL, NULL, ?9, ?9)
         ON CONFLICT(idempotency_key) DO UPDATE SET
             high_watermark_event_id = MAX(COALESCE(extraction_tasks.high_watermark_event_id, 0), excluded.high_watermark_event_id),
             status = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 'pending'
                 ELSE extraction_tasks.status
             END,
             -- Reviving a terminal task resets its retry budget: the old
             -- attempts counted a range the exhaust path already skipped, so
             -- the new range must start with fresh attempts or it would fail
             -- terminally on its first defer.
             attempts = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN 0
                 ELSE extraction_tasks.attempts
             END,
             next_retry_epoch = CASE
                 WHEN extraction_tasks.status IN ('done', 'failed') THEN NULL
                 ELSE extraction_tasks.next_retry_epoch
             END,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            kind.as_str(),
            identity.host_id,
            identity.workspace_id,
            identity.project_id,
            identity.session_row_id,
            kind.priority(),
            idempotency_key,
            event_row_id,
            now
        ],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM extraction_tasks WHERE idempotency_key = ?1",
        params![idempotency_key],
        |row| row.get(0),
    )?)
}

fn exact_hash(content: &str) -> String {
    crate::db::content_identity_hash(content.as_bytes())
}

fn legacy_exact_hash(content: &str) -> String {
    crate::db::legacy_content_identity_hash(content.as_bytes())
}

pub fn unique_capture_event_id(event_type: &str, content: &str) -> String {
    let sanitized_content = redact_capture_content(content);
    let nanos = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    format!(
        "{}-{}-{}",
        event_type,
        nanos,
        exact_hash(&sanitized_content)
    )
}

fn synthesize_event_id(event_type: &str, content_hash: &str) -> String {
    let nanos = chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| chrono::Utc::now().timestamp() * 1_000_000_000);
    format!("{}-{}-{}", event_type, nanos, content_hash)
}

fn estimate_tokens(content: &str) -> i64 {
    ((content.len() as i64) + 3) / 4
}

pub(crate) fn redact_capture_content(content: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        let redacted = crate::adapter::common::redact_sensitive_value(&value);
        return serde_json::to_string(&redacted)
            .unwrap_or_else(|_| crate::adapter::common::redact_sensitive_text(content));
    }
    crate::adapter::common::redact_sensitive_text(content)
}

fn compact_preview(content: &str, max_bytes: usize) -> String {
    let half = (max_bytes / 2).saturating_sub(128);
    let prefix = crate::db::truncate_str(content, half).to_string();
    let suffix_start = content.len().saturating_sub(half);
    let suffix = if content.is_char_boundary(suffix_start) {
        &content[suffix_start..]
    } else {
        let mut start = suffix_start;
        while start < content.len() && !content.is_char_boundary(start) {
            start += 1;
        }
        &content[start..]
    };
    format!(
        "{}\n\n[remem raw event compacted: original_bytes={}]\n\n{}",
        prefix,
        content.len(),
        suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        crate::migrate::run_migrations(&conn).expect("migrations should run");
        conn
    }

    #[test]
    fn record_captured_event_coalesces_extraction_task_by_session() {
        let conn = setup_conn();
        let first = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-1",
                project: "/tmp/remem",
                cwd: Some("/tmp/remem"),
                event_type: "session_stop",
                role: None,
                tool_name: None,
                content: r#"{"session_id":"sess-1"}"#,
                task_kind: Some(ExtractionTaskKind::SessionRollup),
            },
        )
        .expect("first capture should insert");
        let second = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-1",
                project: "/tmp/remem",
                cwd: Some("/tmp/remem"),
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: r#"{"tool_name":"Bash","command":"cargo test"}"#,
                task_kind: Some(ExtractionTaskKind::SessionRollup),
            },
        )
        .expect("second capture should insert");

        assert_eq!(first.extraction_task_id, second.extraction_task_id);
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))
            .unwrap();
        let task_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM extraction_tasks", [], |row| {
                row.get(0)
            })
            .unwrap();
        let high_watermark: i64 = conn
            .query_row(
                "SELECT high_watermark_event_id FROM extraction_tasks",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 2);
        assert_eq!(task_count, 1);
        assert_eq!(high_watermark, second.event_row_id);
    }

    #[test]
    fn large_capture_uses_blob_and_compact_preview() -> Result<()> {
        let conn = setup_conn();
        let content = "x".repeat(DIRECT_CONTENT_BYTES + 2048);
        let outcome = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "claude-code",
                session_id: "sess-large",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Task"),
                content: &content,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
        )?;

        let (retention, blob_id, event_hash): (String, Option<i64>, String) = conn
            .query_row(
                "SELECT retention_class, content_blob_id, content_hash FROM captured_events WHERE id = ?1",
                params![outcome.event_row_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        let blob_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?;
        let blob_hash: String =
            conn.query_row("SELECT content_hash FROM event_blobs", [], |row| row.get(0))?;
        assert_eq!(retention, "raw_compact");
        assert!(blob_id.is_some());
        assert_eq!(blob_count, 1);
        assert!(event_hash.starts_with("sha256:content-v1:"));
        assert!(blob_hash.starts_with("sha256:content-v1:"));
        Ok(())
    }

    #[test]
    fn large_capture_reuses_matching_legacy_blob() -> Result<()> {
        let conn = setup_conn();
        let content = "legacy blob content ".repeat(1200);
        let sanitized_content = redact_capture_content(&content);
        let legacy_hash = legacy_exact_hash(&sanitized_content);
        conn.execute(
            "INSERT INTO event_blobs
             (content_hash, content_encoding, content_bytes, original_bytes, stored_bytes, created_at_epoch)
             VALUES (?1, 'plain', ?2, ?3, ?3, 100)",
            params![
                legacy_hash,
                sanitized_content.as_bytes(),
                sanitized_content.len() as i64
            ],
        )?;
        let legacy_blob_id = conn.last_insert_rowid();

        let outcome = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "claude-code",
                session_id: "sess-legacy-blob",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Task"),
                content: &content,
                task_kind: None,
            },
        )?;

        let (event_hash, blob_id): (String, Option<i64>) = conn.query_row(
            "SELECT content_hash, content_blob_id FROM captured_events WHERE id = ?1",
            params![outcome.event_row_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let blob_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?;
        assert!(event_hash.starts_with("sha256:content-v1:"));
        assert_eq!(blob_id, Some(legacy_blob_id));
        assert_eq!(blob_count, 1);
        Ok(())
    }

    #[test]
    fn large_capture_does_not_reuse_mismatched_legacy_blob() -> Result<()> {
        let conn = setup_conn();
        let content = "target blob content ".repeat(1200);
        let sanitized_content = redact_capture_content(&content);
        let legacy_hash = legacy_exact_hash(&sanitized_content);
        let wrong_content = "different blob content".repeat(1200);
        conn.execute(
            "INSERT INTO event_blobs
             (content_hash, content_encoding, content_bytes, original_bytes, stored_bytes, created_at_epoch)
             VALUES (?1, 'plain', ?2, ?3, ?3, 100)",
            params![
                legacy_hash,
                wrong_content.as_bytes(),
                wrong_content.len() as i64
            ],
        )?;
        let wrong_blob_id = conn.last_insert_rowid();

        let outcome = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "claude-code",
                session_id: "sess-mismatched-legacy-blob",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Task"),
                content: &content,
                task_kind: None,
            },
        )?;

        let blob_id: i64 = conn.query_row(
            "SELECT content_blob_id FROM captured_events WHERE id = ?1",
            params![outcome.event_row_id],
            |row| row.get(0),
        )?;
        let blob_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM event_blobs", [], |row| row.get(0))?;
        assert_ne!(blob_id, wrong_blob_id);
        assert_eq!(blob_count, 2);
        Ok(())
    }

    #[test]
    fn capture_redacts_sensitive_json_before_inline_storage() -> Result<()> {
        let conn = setup_conn();
        let content = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {
                "command": "curl -H 'Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456' https://example.test",
                "api_key": "sk-secret-value"
            },
            "tool_response": {
                "stdout": "password=hunter2\nTOKEN=github_pat_secret"
            }
        })
        .to_string();
        let outcome = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "claude-code",
                session_id: "sess-redact",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &content,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
        )?;

        let stored: String = conn.query_row(
            "SELECT content_text FROM captured_events WHERE id = ?1",
            params![outcome.event_row_id],
            |row| row.get(0),
        )?;
        assert!(stored.contains("[REDACTED]"));
        assert!(!stored.contains("sk-secret-value"));
        assert!(!stored.contains("hunter2"));
        assert!(!stored.contains("github_pat_secret"));
        assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        Ok(())
    }

    #[test]
    fn capture_redacts_sensitive_text_before_blob_storage() -> Result<()> {
        let conn = setup_conn();
        let mut content = "x".repeat(DIRECT_CONTENT_BYTES + 2048);
        content.push_str("\nAuthorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456\n");
        content.push_str("PASSWORD=hunter2\n");
        let outcome = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "claude-code",
                session_id: "sess-redact-blob",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &content,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
        )?;

        let stored: String = conn.query_row(
            "SELECT CAST(b.content_bytes AS TEXT)
                 FROM captured_events e
                 JOIN event_blobs b ON b.id = e.content_blob_id
                 WHERE e.id = ?1",
            params![outcome.event_row_id],
            |row| row.get(0),
        )?;
        assert!(stored.contains("[REDACTED]"));
        assert!(!stored.contains("hunter2"));
        assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        Ok(())
    }

    #[test]
    fn capture_rejects_unknown_host() {
        let conn = setup_conn();
        let err = record_captured_event(
            &conn,
            &CaptureEventInput {
                host: "unknown",
                session_id: "sess-host",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Task"),
                content: "{}",
                task_kind: None,
            },
        )
        .expect_err("unknown host should fail closed");

        assert!(err.to_string().contains("invalid capture host"));
    }
}
