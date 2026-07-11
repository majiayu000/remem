use std::collections::BTreeSet;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::ExtractionTaskKind;
use extraction_task::{
    coalesce_extraction_task, extraction_task_for_replayed_event, with_capture_savepoint,
};

mod extraction_task;

const DIRECT_CONTENT_BYTES: usize = 16 * 1024;

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
    record_captured_event_inner(conn, input, event_id_override, now, now, None, None)
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
        None,
    )
}

pub fn record_captured_event_with_id_and_reference_time_and_git_evidence(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    event_id_override: Option<&str>,
    reference_time_epoch: Option<i64>,
    git_evidence: &[crate::git_util::GitCommitEvidence],
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
        Some(git_evidence),
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
        None,
    )
}

fn record_captured_event_inner(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    event_id_override: Option<&str>,
    created_at_epoch: i64,
    now: i64,
    reference_time_epoch: Option<i64>,
    git_evidence: Option<&[crate::git_util::GitCommitEvidence]>,
) -> Result<CaptureEventOutcome> {
    let inserted_at = now;
    let sanitized_content = redact_capture_content(input.content);
    let content_hash = exact_hash(&sanitized_content);
    let event_id = event_id_override
        .map(ToString::to_string)
        .unwrap_or_else(|| synthesize_event_id(input.event_type, &content_hash));
    let sanitized_git_evidence = git_evidence
        .unwrap_or_default()
        .iter()
        .cloned()
        .map(|mut evidence| {
            evidence.metadata = crate::git_util::sanitize_commit_metadata(evidence.metadata);
            evidence
        })
        .collect::<Vec<_>>();
    let git_branch = input.cwd.and_then(crate::db::detect_git_branch);
    with_capture_savepoint(conn, || {
        let identity = upsert_identity(conn, input, git_branch.as_deref(), now)?;
        let existing_event_row_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM captured_events
             WHERE host_id = ?1 AND session_id = ?2 AND event_id = ?3",
                params![identity.host_id, input.session_id, event_id],
                |row| row.get(0),
            )
            .optional()?;
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

        let mut inserted_git_evidence_keys = BTreeSet::new();
        for evidence in &sanitized_git_evidence {
            let inserted = conn.execute(
                "INSERT INTO captured_event_commits
                 (event_row_id, sha, metadata_json, evidence_kind, evidence_locator)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(event_row_id, sha, evidence_kind) DO NOTHING",
                params![
                    event_row_id,
                    evidence.metadata.sha,
                    serde_json::to_string(&evidence.metadata)?,
                    evidence.kind.as_str(),
                    evidence.locator
                ],
            )?;
            if inserted > 0 {
                inserted_git_evidence_keys.insert(format!(
                    "{}:{}",
                    evidence.kind.as_str(),
                    evidence.metadata.sha.trim().to_ascii_lowercase()
                ));
            }
        }
        let late_git_evidence_key = (!inserted_git_evidence_keys.is_empty()).then(|| {
            exact_hash(
                &inserted_git_evidence_keys
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        });

        let extraction_task_id = if let Some(kind) = input.task_kind {
            if existing_event_row_id.is_some() {
                Some(extraction_task_for_replayed_event(
                    conn,
                    identity,
                    kind,
                    event_row_id,
                    late_git_evidence_key.as_deref(),
                    now,
                )?)
            } else {
                Some(coalesce_extraction_task(
                    conn,
                    identity,
                    kind,
                    event_row_id,
                    now,
                )?)
            }
        } else {
            None
        };

        Ok(CaptureEventOutcome {
            event_row_id,
            event_id,
            extraction_task_id,
        })
    })
}

fn upsert_identity(
    conn: &Connection,
    input: &CaptureEventInput<'_>,
    git_branch: Option<&str>,
    now: i64,
) -> Result<IdentityIds> {
    let host_id = upsert_host(conn, normalize_host(input.host)?, now)?;
    let root_path = input.project.to_string();
    let workspace_id = upsert_workspace(conn, &root_path, git_branch, now)?;
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
        let mut redacted = crate::adapter::common::redact_sensitive_value(&value);
        preserve_capture_path_field(&mut redacted, &value, "cwd");
        preserve_capture_path_field(&mut redacted, &value, "transcript_path");
        return serde_json::to_string(&redacted)
            .unwrap_or_else(|_| crate::adapter::common::redact_sensitive_text(content));
    }
    crate::adapter::common::redact_sensitive_text(content)
}

fn preserve_capture_path_field(
    redacted: &mut serde_json::Value,
    original: &serde_json::Value,
    key: &str,
) {
    let (Some(redacted_obj), Some(original_obj)) = (redacted.as_object_mut(), original.as_object())
    else {
        return;
    };
    if let Some(original_value) = original_obj.get(key).and_then(serde_json::Value::as_str) {
        redacted_obj.insert(
            key.to_string(),
            serde_json::Value::String(original_value.to_string()),
        );
    }
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
#[path = "capture/tests.rs"]
mod tests;
