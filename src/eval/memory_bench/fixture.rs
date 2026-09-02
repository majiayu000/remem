use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::types::{MemoryBenchSuiteFixture, DEFAULT_SUITE_ROOT, SUPPORTED_SUITES};

pub fn suite_path(suite: &str) -> PathBuf {
    Path::new(DEFAULT_SUITE_ROOT).join(suite).join("suite.json")
}

#[cfg(test)]
pub fn load_suite(suite: &str) -> Result<MemoryBenchSuiteFixture> {
    load_suite_with_content_identity(suite).map(|(fixture, _)| fixture)
}

pub(super) fn load_suite_with_content_identity(
    suite: &str,
) -> Result<(MemoryBenchSuiteFixture, String)> {
    if !SUPPORTED_SUITES.contains(&suite) {
        bail!(
            "unknown memory benchmark suite {suite}; supported suites are {}",
            SUPPORTED_SUITES.join(", ")
        );
    }
    load_suite_file_with_content_identity(&suite_path(suite), suite)
}

pub(super) fn load_suite_file_with_content_identity(
    path: &Path,
    requested_suite: &str,
) -> Result<(MemoryBenchSuiteFixture, String)> {
    let bytes = fs::read(path)
        .with_context(|| format!("read memory benchmark suite {}", path.display()))?;
    let fixture: MemoryBenchSuiteFixture = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse memory benchmark suite {}", path.display()))?;
    validate_suite(&fixture)?;
    validate_suite_selection(&fixture, requested_suite)?;
    Ok((fixture, suite_content_identity(&bytes)))
}

fn suite_content_identity(bytes: &[u8]) -> String {
    format!("sha256-raw-suite-v1:{:x}", Sha256::digest(bytes))
}

pub(super) fn validate_suite_selection(
    fixture: &MemoryBenchSuiteFixture,
    requested_suite: &str,
) -> Result<()> {
    if fixture.suite != requested_suite {
        bail!(
            "memory benchmark fixture suite {:?} must match requested suite {:?}",
            fixture.suite,
            requested_suite
        );
    }
    Ok(())
}

pub fn validate_suite(fixture: &MemoryBenchSuiteFixture) -> Result<()> {
    if fixture.schema_version != 1 {
        bail!("memory benchmark suite schema_version must be 1");
    }
    require_non_blank(&fixture.suite, "suite")?;
    require_non_blank(&fixture.version, "version")?;
    require_non_blank(&fixture.fixture_revision, "fixture_revision")?;
    require_non_blank(&fixture.benchmark_id, "benchmark_id")?;
    if fixture.tasks.is_empty() {
        bail!("memory benchmark suite must include tasks");
    }

    let mut task_ids = BTreeSet::new();
    for task in &fixture.tasks {
        if !task_ids.insert(task.id.as_str()) {
            bail!("duplicate memory benchmark task id {}", task.id);
        }
        require_non_blank(&task.id, "task.id")?;
        require_non_blank(&task.category, "task.category")?;
        require_non_blank(&task.prompt, "task.prompt")?;
        require_non_blank(&task.query, "task.query")?;
        require_non_blank(&task.expected_answer, "task.expected_answer")?;
        if task.reference_time_epoch <= 0 {
            bail!("task {} reference_time_epoch must be positive", task.id);
        }
        if task.gold_supporting_event_ids.is_empty() {
            bail!("task {} must include gold_supporting_event_ids", task.id);
        }
        if task.evidence.is_empty() {
            bail!("task {} must include evidence", task.id);
        }

        let evidence_ids = task
            .evidence
            .iter()
            .map(|evidence| evidence.event_id.as_str())
            .collect::<BTreeSet<_>>();
        for event_id in &task.gold_supporting_event_ids {
            if !evidence_ids.contains(event_id.as_str()) {
                bail!(
                    "task {} gold supporting event {} is not present in evidence",
                    task.id,
                    event_id
                );
            }
        }
        for event_id in &task.forbidden_event_ids {
            if !evidence_ids.contains(event_id.as_str()) {
                bail!(
                    "task {} forbidden event {} is not present in evidence",
                    task.id,
                    event_id
                );
            }
        }
        for evidence in &task.evidence {
            require_non_blank(&evidence.event_id, "evidence.event_id")?;
            require_non_blank(&evidence.title, "evidence.title")?;
            require_non_blank(&evidence.content, "evidence.content")?;
            require_non_blank(&evidence.memory_type, "evidence.memory_type")?;
            require_non_blank(&evidence.status, "evidence.status")?;
            require_non_blank(&evidence.scope, "evidence.scope")?;
        }
        if let Some(policy) = &task.policy {
            if policy.non_retention_required && !policy.explicit_approval {
                if policy.expected_active_claims != 0
                    || policy.expected_candidates != 0
                    || policy.expected_summary_inputs != 0
                {
                    bail!(
                        "task {} non-retention policy must expect zero active claims, candidates, and summary inputs",
                        task.id
                    );
                }
                if !policy.expected_policy_abstention {
                    bail!(
                        "task {} non-retention policy must expect policy abstention",
                        task.id
                    );
                }
            }
        }
    }

    Ok(())
}

fn require_non_blank(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} must not be blank");
    }
    Ok(())
}
