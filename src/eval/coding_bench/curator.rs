use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::types::{CodingBenchTask, CuratorLogAttachment};

const MINUTES_PER_SESSION: f64 = 3.0;
const MAX_CHARS: usize = 4_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CuratorLog {
    schema_version: u32,
    condition: String,
    task_id: String,
    target_blind: bool,
    budget: CuratorBudget,
    sessions: Vec<CuratorSession>,
    totals: CuratorTotals,
    final_char_count: usize,
    final_file_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CuratorBudget {
    minutes_per_session: f64,
    max_chars: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CuratorSession {
    episode_id: String,
    minutes_spent: f64,
    edit_count: u64,
    deletion_count: u64,
    conflict_resolution_count: u64,
    chars_after: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CuratorTotals {
    maintenance_minutes: f64,
    update_count: u64,
    deletion_count: u64,
    conflict_resolution_count: u64,
}

pub(super) fn install_budgeted_memory(
    curator_root: &Path,
    task: &CodingBenchTask,
    repo_dir: &Path,
) -> Result<CuratorLogAttachment> {
    let (memory_bytes, attachment) = load_budgeted_input(curator_root, task)?;
    fs::write(repo_dir.join("MEMORY.md"), &memory_bytes)
        .context("write verified budgeted MEMORY.md")?;
    Ok(attachment)
}

pub(super) fn validate_budgeted_input(curator_root: &Path, task: &CodingBenchTask) -> Result<()> {
    load_budgeted_input(curator_root, task).map(|_| ())
}

fn load_budgeted_input(
    curator_root: &Path,
    task: &CodingBenchTask,
) -> Result<(Vec<u8>, CuratorLogAttachment)> {
    let task_root = safe_task_root(curator_root, &task.id)?;
    let memory_path = task_root.join("MEMORY.md");
    let log_path = task_root.join("curator-log.json");
    reject_symlink(&memory_path)?;
    reject_symlink(&log_path)?;
    let memory_bytes = fs::read(&memory_path)
        .with_context(|| format!("read budgeted curator memory {}", memory_path.display()))?;
    let memory_text = std::str::from_utf8(&memory_bytes).with_context(|| {
        format!(
            "budgeted curator memory {} is not UTF-8",
            memory_path.display()
        )
    })?;
    let log_bytes =
        fs::read(&log_path).with_context(|| format!("read curator log {}", log_path.display()))?;
    let log: CuratorLog = serde_json::from_slice(&log_bytes)
        .with_context(|| format!("parse curator log {}", log_path.display()))?;
    let log_sha256 = format!("{:x}", Sha256::digest(&log_bytes));
    let mut attachment = validate_log(task, memory_text, &memory_bytes, log)?;
    attachment.curator_log_sha256 = log_sha256;
    Ok((memory_bytes, attachment))
}

fn validate_log(
    task: &CodingBenchTask,
    memory_text: &str,
    memory_bytes: &[u8],
    log: CuratorLog,
) -> Result<CuratorLogAttachment> {
    if log.schema_version != 1
        || log.condition != "curated_file_budgeted"
        || log.task_id != task.id
        || !log.target_blind
    {
        bail!(
            "curator log identity does not match curated_file_budgeted task {}",
            task.id
        );
    }
    if (log.budget.minutes_per_session - MINUTES_PER_SESSION).abs() > f64::EPSILON
        || log.budget.max_chars != MAX_CHARS
    {
        bail!(
            "curator log for {} does not use the registered 3-minute/4000-character budget",
            task.id
        );
    }
    if log.sessions.len() != task.history_episodes.len() {
        bail!(
            "curator log for {} does not cover every history episode",
            task.id
        );
    }

    let mut minutes = 0.0;
    let mut updates = 0_u64;
    let mut deletions = 0_u64;
    let mut conflicts = 0_u64;
    for (session, episode) in log.sessions.iter().zip(&task.history_episodes) {
        if session.episode_id != episode.episode_id {
            bail!(
                "curator log for {} is not in registered chronological episode order",
                task.id
            );
        }
        if !session.minutes_spent.is_finite()
            || session.minutes_spent < 0.0
            || session.minutes_spent > MINUTES_PER_SESSION
            || session.chars_after > MAX_CHARS
        {
            bail!(
                "curator log for {} exceeds a registered session budget",
                task.id
            );
        }
        minutes += session.minutes_spent;
        updates = updates.saturating_add(session.edit_count);
        deletions = deletions.saturating_add(session.deletion_count);
        conflicts = conflicts.saturating_add(session.conflict_resolution_count);
    }
    if (minutes - log.totals.maintenance_minutes).abs() > 1e-9
        || updates != log.totals.update_count
        || deletions != log.totals.deletion_count
        || conflicts != log.totals.conflict_resolution_count
    {
        bail!(
            "curator log totals do not match per-session accounting for {}",
            task.id
        );
    }

    let char_count = memory_text.chars().count();
    let memory_sha256 = format!("{:x}", Sha256::digest(memory_bytes));
    if char_count != log.final_char_count
        || char_count > MAX_CHARS
        || memory_sha256 != log.final_file_sha256
    {
        bail!(
            "curator log freeze hash/character count mismatch for {}",
            task.id
        );
    }
    Ok(CuratorLogAttachment {
        schema_version: log.schema_version,
        task_id: task.id.clone(),
        target_blind: true,
        memory_sha256,
        curator_log_sha256: String::new(),
        final_char_count: char_count,
        history_session_count: log.sessions.len(),
        maintenance_minutes: minutes,
        update_count: updates,
        deletion_count: deletions,
        conflict_resolution_count: conflicts,
    })
}

fn safe_task_root(root: &Path, task_id: &str) -> Result<PathBuf> {
    if task_id.is_empty()
        || !task_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("invalid task id for curator input path");
    }
    reject_symlink(root)?;
    let task_root = root.join(task_id);
    reject_symlink(&task_root)?;
    Ok(task_root)
}

fn reject_symlink(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect curator input {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("curator input {} must not be a symlink", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_hash_budget_order_and_totals() -> Result<()> {
        let fixture = super::super::fixture::load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let task = &fixture.tasks[0];
        let text = "Ticket normalization uses the registered convention.\n";
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        let log = CuratorLog {
            schema_version: 1,
            condition: "curated_file_budgeted".to_string(),
            task_id: task.id.clone(),
            target_blind: true,
            budget: CuratorBudget {
                minutes_per_session: 3.0,
                max_chars: 4_000,
            },
            sessions: vec![CuratorSession {
                episode_id: task.history_episodes[0].episode_id.clone(),
                minutes_spent: 1.5,
                edit_count: 2,
                deletion_count: 1,
                conflict_resolution_count: 0,
                chars_after: text.chars().count(),
            }],
            totals: CuratorTotals {
                maintenance_minutes: 1.5,
                update_count: 2,
                deletion_count: 1,
                conflict_resolution_count: 0,
            },
            final_char_count: text.chars().count(),
            final_file_sha256: hash,
        };
        let attachment = validate_log(task, text, text.as_bytes(), log)?;
        assert_eq!(attachment.maintenance_minutes, 1.5);
        assert_eq!(attachment.history_session_count, 1);
        Ok(())
    }
}
