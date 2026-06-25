use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use anyhow::{bail, Context, Result};

use super::types::{BenchCondition, CodingBenchFixture, CodingBenchOptions, CodingBenchTask};

const DEFAULT_TASK_SET: &str = "full";
const REQUIRED_CATEGORIES: [&str; 8] = [
    "prior_decision_dependency",
    "prior_bug_root_cause",
    "stale_memory_avoidance",
    "negative_constraints",
    "workstream_continuity",
    "multi_hop_project_context",
    "user_context_relevance",
    "conflict_ambiguity_handling",
];

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
    let task_set = if options.task_set.trim().is_empty() {
        DEFAULT_TASK_SET
    } else {
        options.task_set.trim()
    };
    let filtered = fixture
        .tasks
        .iter()
        .filter(|task| match task_set {
            "full" | "v1" => true,
            "smoke" => task.smoke,
            _ => false,
        })
        .collect::<Vec<_>>();
    if !matches!(task_set, "full" | "v1" | "smoke") {
        bail!("unknown coding benchmark task set: {task_set}");
    }
    match options.task.as_deref() {
        Some(id) => filtered
            .into_iter()
            .find(|task| task.id == id)
            .map(|task| vec![task])
            .ok_or_else(|| anyhow::anyhow!("unknown benchmark task: {id}")),
        None => Ok(filtered),
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
    if fixture
        .repo
        .base_commit
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        bail!("coding benchmark fixture repo.base_commit must pin the task repository base");
    }
    if fixture
        .repo
        .fixture_revision
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        bail!("coding benchmark fixture repo.fixture_revision must not be blank");
    }
    for path in fixture.repo.files.keys() {
        validate_relative_path(path)?;
    }
    if !(12..=20).contains(&fixture.tasks.len()) {
        bail!(
            "coding benchmark fixture must contain 12-20 tasks, found {}",
            fixture.tasks.len()
        );
    }

    let mut task_ids = BTreeSet::new();
    let mut category_counts = BTreeMap::<String, usize>::new();
    let mut smoke_count = 0usize;
    for task in &fixture.tasks {
        if task.id.trim().is_empty() {
            bail!("coding benchmark task id must not be blank");
        }
        if !task_ids.insert(task.id.clone()) {
            bail!("duplicate coding benchmark task id {}", task.id);
        }
        if task.category.trim().is_empty() {
            bail!("task {} category must not be blank", task.id);
        }
        *category_counts.entry(task.category.clone()).or_default() += 1;
        if task.smoke {
            smoke_count += 1;
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
        for path in &task.forbidden_paths {
            validate_relative_path(path)?;
        }
        for path in task.score.hidden_files.keys() {
            validate_relative_path(path)?;
        }
        for pattern in task
            .score
            .required_patch_patterns
            .iter()
            .chain(task.score.forbidden_patch_patterns.iter())
        {
            if pattern.trim().is_empty() {
                bail!("task {} contains a blank patch pattern", task.id);
            }
        }
        if task.history_episodes.is_empty() {
            bail!("task {} must define history_episodes", task.id);
        }
        for episode in &task.history_episodes {
            if episode.episode_id.trim().is_empty() {
                bail!("task {} contains a blank history episode id", task.id);
            }
            if episode.reference_time_epoch <= 0 {
                bail!(
                    "task {} episode {} reference_time_epoch must be positive",
                    task.id,
                    episode.episode_id
                );
            }
            if episode.summary.trim().is_empty() {
                bail!(
                    "task {} episode {} summary must not be blank",
                    task.id,
                    episode.episode_id
                );
            }
            if episode.expected_memory_facts.is_empty() {
                bail!(
                    "task {} episode {} must list expected_memory_facts",
                    task.id,
                    episode.episode_id
                );
            }
            for memory in &episode.memories {
                validate_memory(&task.id, memory)?;
            }
        }
        for memory in &task.memories {
            validate_memory(&task.id, memory)?;
        }
        if task.seed_memories().is_empty() {
            bail!("task {} must seed at least one memory", task.id);
        }
        if task.gold_memory.required_facts.is_empty() {
            bail!(
                "task {} gold_memory.required_facts must not be empty",
                task.id
            );
        }
        if task.gold_memory.supporting_event_ids.is_empty() {
            bail!(
                "task {} gold_memory.supporting_event_ids must not be empty",
                task.id
            );
        }
        for fact in task
            .gold_memory
            .required_facts
            .iter()
            .chain(task.gold_memory.forbidden_facts.iter())
        {
            if fact.trim().is_empty() {
                bail!("task {} gold_memory contains a blank fact id", task.id);
            }
        }
    }
    if smoke_count == 0 {
        bail!("coding benchmark fixture must mark at least one smoke task");
    }
    for category in REQUIRED_CATEGORIES {
        let count = category_counts.get(category).copied().unwrap_or(0);
        if count < 2 {
            bail!("coding benchmark category {category} must have at least 2 tasks, found {count}");
        }
    }
    Ok(())
}

fn validate_memory(task_id: &str, memory: &super::types::SeedMemory) -> Result<()> {
    if memory.title.trim().is_empty() {
        bail!("task {} contains a memory with a blank title", task_id);
    }
    if memory.text.trim().is_empty() {
        bail!("task {} contains a memory with blank text", task_id);
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
