use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const ORPHANED_SUMMARY_SPILL_CLAIM_MIN_AGE_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SummaryHookSpillRecord {
    version: u32,
    pub(super) input: String,
    pub(super) host: Option<String>,
    pub(super) profile: Option<String>,
    #[serde(default)]
    pub(super) git_evidence: Vec<crate::git_util::GitCommitEvidence>,
    db_error: String,
    created_at_epoch: i64,
}

pub(super) fn spill_summary_hook_payload(
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
    resolved_cwd: Option<&str>,
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    spill_summary_hook_payload_with_git_evidence(input, host, profile, resolved_cwd, &[], db_error)
}

pub(super) fn spill_summary_hook_payload_with_git_evidence(
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
    resolved_cwd: Option<&str>,
    git_evidence: &[crate::git_util::GitCommitEvidence],
    db_error: &anyhow::Error,
) -> Result<PathBuf> {
    let path = summary_spill_path();
    let record = SummaryHookSpillRecord {
        version: 2,
        input: summary_spill_input(input, resolved_cwd, profile)?,
        host: host.map(crate::runtime_config::normalize_host),
        profile: profile.map(str::to_string),
        git_evidence: git_evidence
            .iter()
            .cloned()
            .map(|mut evidence| {
                evidence.metadata = crate::git_util::sanitize_commit_metadata(evidence.metadata);
                evidence
            })
            .collect(),
        db_error: crate::db::truncate_str(
            &crate::db::capture::redact_capture_content(&db_error.to_string()),
            1000,
        )
        .to_string(),
        created_at_epoch: chrono::Utc::now().timestamp(),
    };
    append_record_to_spill(&path, &record)?;
    Ok(path)
}

pub(super) fn replay_spilled_summary_hook_payloads(
    conn: &Connection,
    mut replay: impl FnMut(&Connection, &SummaryHookSpillRecord) -> Result<()>,
) -> Result<usize> {
    let path = summary_spill_path();
    let queue = crate::spill_queue::SpillQueue::new(path)?;
    let Some(claim) = queue.claim(Duration::from_secs(
        ORPHANED_SUMMARY_SPILL_CLAIM_MIN_AGE_SECS,
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
    let mut replayed = 0;
    let result = (|| -> Result<()> {
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            match crate::db::spill_crypto::decode_json_line::<SummaryHookSpillRecord>(line) {
                Ok(record) => match replay(conn, &record) {
                    Ok(()) => replayed += 1,
                    Err(error) => append_failed_record(claim.failed_path(), &record, &error)?,
                },
                Err(error) => append_failed_line(claim.failed_path(), line, &error)?,
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
            "summarize",
            &format!("replayed {replayed} spilled summary hook payload(s)"),
        );
    }
    Ok(replayed)
}

fn summary_spill_input(
    input: &str,
    resolved_cwd: Option<&str>,
    profile: Option<&str>,
) -> Result<String> {
    let mut payload: serde_json::Value = serde_json::from_str(input)?;
    let Some(obj) = payload.as_object_mut() else {
        return Ok(input.to_string());
    };
    if let Some(cwd) = resolved_cwd.filter(|cwd| !cwd.trim().is_empty()) {
        let needs_cwd = obj
            .get("cwd")
            .and_then(|value| value.as_str())
            .is_none_or(|value| value.trim().is_empty());
        if needs_cwd {
            obj.insert(
                "cwd".to_string(),
                serde_json::Value::String(cwd.to_string()),
            );
        }
    }
    if let Some(profile) = profile.map(str::trim).filter(|profile| !profile.is_empty()) {
        obj.insert(
            crate::runtime_config::MEMORY_AI_PROFILE_FIELD.to_string(),
            serde_json::Value::String(profile.to_string()),
        );
    }
    Ok(serde_json::to_string(&payload)?)
}

fn append_record_to_spill(path: &Path, record: &SummaryHookSpillRecord) -> Result<()> {
    let line = crate::db::spill_crypto::encode_json_line(record)?;
    crate::spill_queue::SpillQueue::new(path.to_path_buf())?.append_line(line.as_bytes())
}

fn append_failed_line(path: &Path, line: &str, error: &anyhow::Error) -> Result<()> {
    crate::spill_queue::SpillQueue::new(path.to_path_buf())?.append_line(line.as_bytes())?;
    crate::log::warn(
        "summarize",
        &format!("summary hook spill replay failed: {error}"),
    );
    Ok(())
}

fn append_failed_record(
    path: &Path,
    record: &SummaryHookSpillRecord,
    error: &anyhow::Error,
) -> Result<()> {
    append_record_to_spill(path, record)?;
    crate::log::warn(
        "summarize",
        &format!("summary hook spill replay failed: {error}"),
    );
    Ok(())
}

pub(super) fn summary_spill_path() -> PathBuf {
    crate::db::data_dir().join("summary-hook-spill.jsonl")
}

#[cfg(test)]
fn failed_summary_spill_path_for_claim(claimed_path: &Path) -> PathBuf {
    claimed_path.with_extension("failed.jsonl")
}

#[cfg(test)]
fn restore_claimed_and_failed_spill(claimed_path: &Path, _failed_path: &Path, path: &Path) {
    let queue = crate::spill_queue::SpillQueue::new(path.to_path_buf())
        .expect("summary spill test queue should be valid");
    queue
        .adopt_claim(claimed_path.to_path_buf())
        .restore()
        .expect("summary spill test claim should restore");
}

#[cfg(test)]
fn restore_orphaned_summary_spill_claims(min_age: Duration) -> Result<usize> {
    crate::spill_queue::SpillQueue::new(summary_spill_path())?.restore_orphaned_claims(min_age)
}

#[cfg(test)]
fn claimed_summary_spill_path() -> PathBuf {
    crate::spill_queue::SpillQueue::new(summary_spill_path())
        .expect("summary spill test queue should be valid")
        .next_claim_path()
}

#[cfg(test)]
#[path = "spill/tests.rs"]
mod tests;
