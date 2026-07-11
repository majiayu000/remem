use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::git_util::{GitCommitEvidence, GitCommitMetadata, GitEvidenceKind};

pub(super) const SPILL_REASON_DB_OPEN_FAILED: &str = "db_open_failed";
pub(super) const SPILL_REASON_CAPTURE_PERSISTENCE_FAILED: &str = "capture_persistence_failed";
const ORPHANED_CAPTURE_SPILL_CLAIM_MIN_AGE_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CaptureSpillRecord {
    version: u32,
    event_id: String,
    host: String,
    event: ParsedHookEvent,
    summary: EventSummary,
    #[serde(default)]
    git_evidence: Vec<GitCommitEvidence>,
    failure_reason: String,
    db_error: String,
    created_at_epoch: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct CaptureSpillRecordCompat {
    version: u32,
    event_id: Option<String>,
    host: String,
    event: ParsedHookEvent,
    summary: EventSummary,
    #[serde(default)]
    git_metadata: Option<GitCommitMetadata>,
    #[serde(default)]
    git_evidence: Vec<GitCommitEvidence>,
    failure_reason: Option<String>,
    db_error: String,
    created_at_epoch: i64,
}

impl CaptureSpillRecordCompat {
    fn into_record(self, fallback_event_id: String) -> CaptureSpillRecord {
        let mut event = sanitize_event(&self.event);
        event.reference_time_epoch = event.reference_time_epoch.or(Some(self.created_at_epoch));
        let mut git_evidence = self.git_evidence;
        if git_evidence.is_empty() {
            if let Some(metadata) = self.git_metadata {
                git_evidence.push(GitCommitEvidence {
                    kind: GitEvidenceKind::TerminalSnapshot,
                    metadata,
                    locator: Some("legacy_observe_spill".to_string()),
                });
            }
        }
        CaptureSpillRecord {
            version: self.version,
            event_id: self.event_id.unwrap_or(fallback_event_id),
            host: self.host,
            event,
            summary: sanitize_summary(&self.summary),
            git_evidence,
            failure_reason: self
                .failure_reason
                .unwrap_or_else(|| SPILL_REASON_DB_OPEN_FAILED.to_string()),
            db_error: crate::db::truncate_str(
                &crate::db::capture::redact_capture_content(&self.db_error),
                1000,
            )
            .to_string(),
            created_at_epoch: self.created_at_epoch,
        }
    }
}

pub(super) fn record_capture_drop_lossy(
    host: Option<&str>,
    event: Option<&ParsedHookEvent>,
    reason: &str,
    detail: Option<&str>,
) {
    let Ok(conn) = crate::db::open_db_for_hook() else {
        crate::log::warn(
            "observe",
            &format!("capture drop could not be recorded: reason={reason}"),
        );
        return;
    };
    let result = crate::db::record_capture_drop(
        &conn,
        &crate::db::CaptureDropInput {
            host,
            session_id: event.map(|event| event.session_id.as_str()),
            project: event.map(|event| event.project.as_str()),
            tool_name: event.map(|event| event.tool_name.as_str()),
            reason,
            detail,
            spill_path: None,
            recovered_event_id: None,
        },
    );
    if let Err(error) = result {
        crate::log::warn(
            "observe",
            &format!("capture drop ledger write failed: {error}"),
        );
    }
}

#[cfg(test)]
pub(super) fn spill_capture_event(
    host: &str,
    event_id: &str,
    event: &ParsedHookEvent,
    summary: &EventSummary,
    failure_reason: &str,
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    spill_capture_event_with_git_evidence(
        host,
        event_id,
        event,
        summary,
        &[],
        failure_reason,
        db_error,
    )
}

pub(super) fn spill_capture_event_with_git_evidence(
    host: &str,
    event_id: &str,
    event: &ParsedHookEvent,
    summary: &EventSummary,
    git_evidence: &[GitCommitEvidence],
    failure_reason: &str,
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    let path = spill_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create capture spill dir {}", parent.display()))?;
    }
    let created_at_epoch = event
        .reference_time_epoch
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let record = CaptureSpillRecord {
        version: 2,
        event_id: event_id.to_string(),
        host: host.to_string(),
        event: sanitize_event(event),
        summary: sanitize_summary(summary),
        git_evidence: git_evidence
            .iter()
            .cloned()
            .map(|mut evidence| {
                evidence.metadata = crate::git_util::sanitize_commit_metadata(evidence.metadata);
                evidence
            })
            .collect(),
        failure_reason: failure_reason.to_string(),
        db_error: crate::db::truncate_str(
            &crate::db::capture::redact_capture_content(&db_error.to_string()),
            1000,
        )
        .to_string(),
        created_at_epoch,
    };
    append_spill_record(&path, &record)?;
    Ok(path)
}

pub(super) fn replay_spilled_capture_events(conn: &Connection) -> Result<usize> {
    let path = spill_path();
    let queue = crate::spill_queue::SpillQueue::new(path.clone())?;
    let Some(claim) = queue.claim(Duration::from_secs(
        ORPHANED_CAPTURE_SPILL_CLAIM_MIN_AGE_SECS,
    ))?
    else {
        return Ok(0);
    };
    let contents = match std::fs::read_to_string(claim.path()) {
        Ok(contents) => contents,
        Err(error) => {
            claim.restore()?;
            return Err(error).with_context(|| format!("read {}", claim.path().display()));
        }
    };
    let contents = match normalize_claimed_spill_records(claim.path(), &contents) {
        Ok(contents) => contents,
        Err(error) => {
            claim.restore()?;
            return Err(error).with_context(|| format!("normalize {}", claim.path().display()));
        }
    };
    let mut replayed = 0;
    let result = (|| -> Result<()> {
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            match parse_spill_record(line) {
                Ok(record) => match replay_spill_record(conn, &path, &record) {
                    Ok(true) => replayed += 1,
                    Ok(false) => {}
                    Err(error) => append_failed_spill_record(claim.failed_path(), &record, &error)?,
                },
                Err(error) => append_failed_spill_line(claim.failed_path(), line, &error)?,
            }
        }
        claim.finish()
    })();
    if let Err(error) = result {
        claim.restore()?;
        return Err(error);
    }

    if replayed > 0 {
        crate::log::info(
            "observe",
            &format!("replayed {replayed} spilled capture event(s)"),
        );
    }
    Ok(replayed)
}

fn normalize_claimed_spill_records(path: &Path, contents: &str) -> Result<String> {
    let mut changed = false;
    let mut normalized_lines = Vec::new();
    for (line_index, line) in contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let normalized_line =
            match crate::db::spill_crypto::decode_json_line::<CaptureSpillRecordCompat>(line) {
                Ok(record) if record.event_id.is_none() => {
                    changed = true;
                    let event_id = unique_legacy_spill_event_id(line)?;
                    crate::db::spill_crypto::encode_json_line(&record.into_record(event_id))
                        .with_context(|| {
                            format!("encode legacy capture spill line {}", line_index + 1)
                        })?
                }
                _ => line.to_string(),
            };
        normalized_lines.push(normalized_line);
    }
    if !changed {
        return Ok(contents.to_string());
    }
    let mut normalized = normalized_lines.join("\n");
    if !normalized.is_empty() {
        normalized.push('\n');
    }
    crate::atomic_file::write_atomic(path, normalized.as_bytes())
        .with_context(|| format!("persist normalized capture spill {}", path.display()))?;
    Ok(normalized)
}

fn replay_spill_record(
    conn: &Connection,
    spill_path: &Path,
    record: &CaptureSpillRecord,
) -> Result<bool> {
    if recovered_spill_exists(conn, record)? {
        return Ok(false);
    }

    let tx = conn
        .unchecked_transaction()
        .context("start capture spill replay transaction")?;
    let event_id = super::hook::record_observed_event_with_id(
        &tx,
        &record.host,
        &record.event_id,
        &record.event,
        &record.summary,
        &record.git_evidence,
    )?;
    let spill_path = spill_path.display().to_string();
    let drop_input = crate::db::CaptureDropInput {
        host: Some(&record.host),
        session_id: Some(&record.event.session_id),
        project: Some(&record.event.project),
        tool_name: Some(&record.event.tool_name),
        reason: &record.failure_reason,
        detail: Some(&record.db_error),
        spill_path: Some(&spill_path),
        recovered_event_id: Some(event_id),
    };
    if !crate::db::mark_capture_spill_recovered(&tx, &drop_input, event_id)? {
        crate::db::record_capture_drop(&tx, &drop_input)?;
    }
    tx.commit()
        .context("commit capture spill replay transaction")?;
    Ok(true)
}

fn append_failed_spill_line(path: &Path, line: &str, error: &anyhow::Error) -> Result<()> {
    crate::spill_queue::SpillQueue::new(path.to_path_buf())?.append_line(line.as_bytes())?;
    crate::log::warn("observe", &format!("capture spill replay failed: {error}"));
    Ok(())
}

fn append_failed_spill_record(
    path: &Path,
    record: &CaptureSpillRecord,
    error: &anyhow::Error,
) -> Result<()> {
    append_spill_record(path, record)?;
    crate::log::warn("observe", &format!("capture spill replay failed: {error}"));
    Ok(())
}

fn append_spill_record(path: &Path, record: &CaptureSpillRecord) -> Result<()> {
    let line = crate::db::spill_crypto::encode_json_line(record)?;
    crate::spill_queue::SpillQueue::new(path.to_path_buf())?.append_line(line.as_bytes())
}

fn sanitize_event(event: &ParsedHookEvent) -> ParsedHookEvent {
    ParsedHookEvent {
        session_id: event.session_id.clone(),
        cwd: event.cwd.clone(),
        project: event.project.clone(),
        reference_time_epoch: event.reference_time_epoch,
        tool_name: event.tool_name.clone(),
        tool_input: event
            .tool_input
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
        tool_response: event
            .tool_response
            .as_ref()
            .map(crate::adapter::common::redact_sensitive_value),
    }
}

fn sanitize_summary(summary: &EventSummary) -> EventSummary {
    EventSummary {
        event_type: summary.event_type.clone(),
        summary: crate::db::capture::redact_capture_content(&summary.summary),
        detail: summary
            .detail
            .as_ref()
            .map(|detail| crate::db::capture::redact_capture_content(detail)),
        files_json: summary.files_json.clone(),
        exit_code: summary.exit_code,
    }
}

fn parse_spill_record(line: &str) -> Result<CaptureSpillRecord> {
    let record: CaptureSpillRecordCompat = crate::db::spill_crypto::decode_json_line(line)?;
    let event_id = record
        .event_id
        .clone()
        .context("capture spill record is missing normalized event_id")?;
    Ok(record.into_record(event_id))
}

fn recovered_spill_exists(conn: &Connection, record: &CaptureSpillRecord) -> Result<bool> {
    let exists = conn
        .query_row(
            "SELECT 1
             FROM captured_events captured
             JOIN hosts ON hosts.id = captured.host_id
             JOIN capture_drop_events drop_event
               ON drop_event.recovered_event_id = captured.id
             WHERE hosts.name = ?1
               AND captured.session_id = ?2
               AND captured.event_id = ?3
               AND drop_event.reason = ?4
             LIMIT 1",
            rusqlite::params![
                &record.host,
                &record.event.session_id,
                &record.event_id,
                &record.failure_reason
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(exists)
}

fn unique_legacy_spill_event_id(line: &str) -> Result<String> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| anyhow::anyhow!("generate legacy capture spill identity: {error}"))?;
    let nonce = u128::from_ne_bytes(nonce);
    Ok(format!(
        "tool_result-legacy-spill-{:016x}-{nonce:032x}",
        crate::db::deterministic_hash(line.as_bytes())
    ))
}

fn spill_path() -> PathBuf {
    crate::db::data_dir().join("capture-spill.jsonl")
}

#[cfg(test)]
#[path = "spill/tests.rs"]
mod tests;
