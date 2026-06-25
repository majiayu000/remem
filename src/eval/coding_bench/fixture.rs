use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};

use anyhow::{bail, Context, Result};

use super::types::{BenchCondition, CodingBenchFixture, CodingBenchOptions, CodingBenchTask};

pub fn load_fixture(path: &str) -> Result<CodingBenchFixture> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read coding benchmark fixture {path}"))?;
    let fixture: CodingBenchFixture =
        serde_json::from_str(&content).with_context(|| format!("parse fixture {path}"))?;
    validate_fixture(&fixture)?;
    Ok(fixture)
}

pub fn selected_conditions(options: &CodingBenchOptions) -> Result<Vec<BenchCondition>> {
    match options.condition.as_deref() {
        Some(value) => BenchCondition::parse(value)
            .map(|condition| vec![condition])
            .ok_or_else(|| anyhow::anyhow!("unknown benchmark condition: {value}")),
        None => Ok(BenchCondition::ALL.to_vec()),
    }
}

pub fn selected_tasks<'a>(
    fixture: &'a CodingBenchFixture,
    options: &CodingBenchOptions,
) -> Result<Vec<&'a CodingBenchTask>> {
    match options.task.as_deref() {
        Some(id) => fixture
            .tasks
            .iter()
            .find(|task| task.id == id)
            .map(|task| vec![task])
            .ok_or_else(|| anyhow::anyhow!("unknown benchmark task: {id}")),
        None => Ok(fixture.tasks.iter().collect()),
    }
}

fn validate_fixture(fixture: &CodingBenchFixture) -> Result<()> {
    if fixture.version != 1 {
        bail!(
            "unsupported coding benchmark fixture version {}",
            fixture.version
        );
    }
    if fixture.repo.kind != "inline_python" {
        bail!(
            "unsupported coding benchmark repo kind {}; expected inline_python",
            fixture.repo.kind
        );
    }
    if fixture.repo.files.is_empty() {
        bail!("coding benchmark fixture repo.files must not be empty");
    }
    for path in fixture.repo.files.keys() {
        validate_relative_path(path)?;
    }
    if fixture.tasks.is_empty() {
        bail!("coding benchmark fixture must contain at least one task");
    }

    let mut task_ids = BTreeSet::new();
    for task in &fixture.tasks {
        if task.id.trim().is_empty() {
            bail!("coding benchmark task id must not be blank");
        }
        if !task_ids.insert(task.id.clone()) {
            bail!("duplicate coding benchmark task id {}", task.id);
        }
        if task.prompt.trim().is_empty() {
            bail!("task {} prompt must not be blank", task.id);
        }
        if task.score.commands.is_empty() {
            bail!("task {} must define at least one score command", task.id);
        }
        for command in &task.score.commands {
            if command.is_empty() || command[0].trim().is_empty() {
                bail!("task {} contains an empty score command", task.id);
            }
        }
        for path in &task.allowed_paths {
            validate_relative_path(path)?;
        }
        for path in task.score.hidden_files.keys() {
            validate_relative_path(path)?;
        }
        for memory in &task.memories {
            if memory.title.trim().is_empty() {
                bail!("task {} contains a memory with a blank title", task.id);
            }
            if memory.text.trim().is_empty() {
                bail!("task {} contains a memory with blank text", task.id);
            }
        }
    }
    Ok(())
}

pub fn validate_relative_path(path: &str) -> Result<()> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        bail!("relative path must not be blank");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("path {} must be a safe relative path", path.display()),
        }
    }
    Ok(())
}
