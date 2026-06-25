use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::types::{MemoryBenchSuiteFixture, DEFAULT_SUITE, DEFAULT_SUITE_ROOT};

pub fn suite_path(suite: &str) -> PathBuf {
    Path::new(DEFAULT_SUITE_ROOT).join(suite).join("suite.json")
}

pub fn load_suite(suite: &str) -> Result<MemoryBenchSuiteFixture> {
    if suite != DEFAULT_SUITE {
        bail!("unknown memory benchmark suite {suite}; supported suite is {DEFAULT_SUITE}");
    }
    let path = suite_path(suite);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("read memory benchmark suite {}", path.display()))?;
    let fixture: MemoryBenchSuiteFixture = serde_json::from_str(&content)
        .with_context(|| format!("parse memory benchmark suite {}", path.display()))?;
    validate_suite(&fixture)?;
    Ok(fixture)
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
    }

    Ok(())
}

fn require_non_blank(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} must not be blank");
    }
    Ok(())
}
