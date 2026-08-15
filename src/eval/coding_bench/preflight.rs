use std::path::Path;

use anyhow::{Context, Result};

use super::types::{BenchCondition, CodingBenchOptions, CodingBenchTask};

pub(super) fn validate_condition_inputs(
    options: &CodingBenchOptions,
    conditions: &[BenchCondition],
    tasks: &[&CodingBenchTask],
) -> Result<()> {
    if conditions.contains(&BenchCondition::CuratedFileBudgeted) {
        let root = options
            .curator_root
            .as_deref()
            .map(Path::new)
            .context("curated_file_budgeted requires --curator-root before any run starts")?;
        for task in tasks {
            super::curator::validate_budgeted_input(root, task)
                .with_context(|| format!("preflight curated_file_budgeted task {}", task.id))?;
        }
    }
    if conditions.contains(&BenchCondition::RememE2e) {
        let path = options
            .memory_config
            .as_deref()
            .map(Path::new)
            .context("remem_e2e requires --memory-config before any run starts")?;
        super::e2e::validate_memory_config(path).context("preflight remem_e2e memory config")?;
    }
    Ok(())
}
