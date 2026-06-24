use std::ffi::OsString;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::types::{BenchCondition, CodingBenchFixture, CodingBenchTask, SeedMemory};

#[derive(Debug, Clone)]
pub struct ConditionSetup {
    pub env: Vec<(String, String)>,
    pub prompt_note: Option<String>,
}

pub fn apply_condition(
    condition: BenchCondition,
    fixture: &CodingBenchFixture,
    task: &CodingBenchTask,
    repo_dir: &Path,
    data_dir: &Path,
) -> Result<ConditionSetup> {
    match condition {
        BenchCondition::NoMemory => Ok(ConditionSetup {
            env: vec![("REMEM_DISABLE_HOOKS".to_string(), "1".to_string())],
            prompt_note: None,
        }),
        BenchCondition::CuratedFile => {
            let content = task
                .curated_context
                .as_deref()
                .or(fixture.curated_context.as_deref())
                .context("curated_file condition requires curated_context in fixture or task")?;
            fs::write(repo_dir.join("MEMORY.md"), content).context("write curated MEMORY.md")?;
            Ok(ConditionSetup {
                env: vec![("REMEM_DISABLE_HOOKS".to_string(), "1".to_string())],
                prompt_note: Some(
                    "A curated MEMORY.md file is available in the repo. Read it before editing."
                        .to_string(),
                ),
            })
        }
        BenchCondition::Remem => {
            let rendered = render_seeded_remem_context(data_dir, repo_dir, task)?;
            fs::write(repo_dir.join("REMEM_CONTEXT.md"), rendered)
                .context("write remem benchmark context")?;
            Ok(ConditionSetup {
                env: vec![
                    (
                        "REMEM_DATA_DIR".to_string(),
                        data_dir.to_string_lossy().to_string(),
                    ),
                    ("REMEM_ALLOW_PLAINTEXT_DB".to_string(), "1".to_string()),
                ],
                prompt_note: Some(
                    "A remem SessionStart context file is available at REMEM_CONTEXT.md. Read it before editing; if it contains Benchmark Memory Details, use those preloaded remem details before calling any memory tool."
                        .to_string(),
                ),
            })
        }
    }
}

pub fn render_seeded_remem_context(
    data_dir: &Path,
    repo_dir: &Path,
    task: &CodingBenchTask,
) -> Result<String> {
    fs::create_dir_all(data_dir).context("create benchmark REMEM_DATA_DIR")?;
    let _env = ScopedEnvVars::set_many([
        ("REMEM_DATA_DIR", data_dir.as_os_str().to_os_string()),
        ("REMEM_ALLOW_PLAINTEXT_DB", OsString::from("1")),
    ]);
    let conn = crate::db::open_db().context("open benchmark remem database")?;
    let project = repo_dir.to_string_lossy().to_string();
    let mut seeded_ids = Vec::new();
    for memory in &task.memories {
        let saved = save_seed_memory(&conn, &project, memory)?;
        seeded_ids.push(saved.id);
    }
    let seeded_memories = crate::memory::get_memories_by_ids(&conn, &seeded_ids, Some(&project))
        .context("load seeded benchmark memories")?;
    if seeded_memories.len() != seeded_ids.len() {
        bail!(
            "seeded {} remem memories but only {} are visible to project {}",
            seeded_ids.len(),
            seeded_memories.len(),
            project
        );
    }
    let snapshot =
        crate::context::session_start_eval_snapshot(&project, &project, Some("main"), "codex-cli")
            .context("render benchmark SessionStart context")?;
    let mut output = snapshot.rendered_output;
    append_benchmark_memory_details(&mut output, &seeded_memories);
    Ok(output)
}

fn save_seed_memory(
    conn: &rusqlite::Connection,
    project: &str,
    memory: &SeedMemory,
) -> Result<crate::memory::service::SaveMemoryResult> {
    crate::memory::service::save_memory(
        conn,
        &crate::memory::service::SaveMemoryRequest {
            text: memory.text.clone(),
            title: Some(memory.title.clone()),
            project: Some(project.to_string()),
            session_id: Some("coding-bench-seed".to_string()),
            host: Some("codex-cli".to_string()),
            topic_key: memory.topic_key.clone(),
            memory_type: memory.memory_type.clone(),
            files: if memory.files.is_empty() {
                None
            } else {
                Some(memory.files.clone())
            },
            scope: Some("project".to_string()),
            local_copy_enabled: Some(false),
            claim_enabled: Some(false),
            ..Default::default()
        },
    )
}

fn append_benchmark_memory_details(output: &mut String, memories: &[crate::memory::Memory]) {
    if memories.is_empty() {
        return;
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("\n## Benchmark Memory Details\n");
    output.push_str(
        "The entries below are full memory details loaded from the same temporary remem database used for this benchmark run. Treat them as preloaded get_observations results.\n\n",
    );
    for memory in memories {
        output.push_str(&format!("### memory:#{} {}\n", memory.id, memory.title));
        output.push_str(&format!("- type: {}\n", memory.memory_type));
        if let Some(topic_key) = &memory.topic_key {
            output.push_str(&format!("- topic_key: {topic_key}\n"));
        }
        if let Some(files) = &memory.files {
            output.push_str(&format!("- files: {files}\n"));
        }
        output.push('\n');
        output.push_str(memory.text.trim_end());
        output.push_str("\n\n");
    }
}

struct ScopedEnvVars {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedEnvVars {
    fn set_many<const N: usize>(values: [(&'static str, OsString); N]) -> Self {
        let previous = values
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for (key, value) in values {
            std::env::set_var(key, value);
        }
        Self { previous }
    }
}

impl Drop for ScopedEnvVars {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remem_context_preloads_full_detail_for_indexed_memory() {
        let mut rendered =
            "remem context\n\n## Index\n**Procedures** (1): #1 Slug normalizer contract\n"
                .to_string();
        append_benchmark_memory_details(
            &mut rendered,
            &[crate::memory::Memory {
                id: 1,
                session_id: Some("coding-bench-seed".to_string()),
                project: "/tmp/repo".to_string(),
                topic_key: Some("slug-normalizer".to_string()),
                title: "Slug normalizer contract".to_string(),
                text: "Empty slug output must be `untitled`.".to_string(),
                memory_type: "procedure".to_string(),
                files: Some("[\"memory_demo/slug.py\"]".to_string()),
                created_at_epoch: 1,
                updated_at_epoch: 1,
                status: "active".to_string(),
                branch: None,
                scope: "project".to_string(),
            }],
        );

        assert!(rendered.contains("## Index"));
        assert!(rendered.contains("## Benchmark Memory Details"));
        assert!(rendered.contains("Slug normalizer contract"));
        assert!(rendered.contains("`untitled`"));
    }
}
