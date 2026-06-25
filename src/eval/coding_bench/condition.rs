use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::types::{
    BenchCondition, CodingBenchFixture, CodingBenchTask, CodingMemoryAttributionInput, SeedMemory,
};

#[derive(Debug, Clone)]
pub struct ConditionSetup {
    pub env: Vec<(String, String)>,
    pub prompt_note: Option<String>,
    pub memory_attribution: CodingMemoryAttributionInput,
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
            memory_attribution: CodingMemoryAttributionInput::default(),
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
                memory_attribution: CodingMemoryAttributionInput::default(),
            })
        }
        BenchCondition::Remem => {
            let (rendered, memory_attribution) =
                render_seeded_remem_context(data_dir, repo_dir, task)?;
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
                memory_attribution,
            })
        }
    }
}

pub fn render_seeded_remem_context(
    data_dir: &Path,
    repo_dir: &Path,
    task: &CodingBenchTask,
) -> Result<(String, CodingMemoryAttributionInput)> {
    fs::create_dir_all(data_dir).context("create benchmark REMEM_DATA_DIR")?;
    let _env = ScopedEnvVars::set_many([
        ("REMEM_DATA_DIR", data_dir.as_os_str().to_os_string()),
        ("REMEM_ALLOW_PLAINTEXT_DB", OsString::from("1")),
    ]);
    let conn = crate::db::open_db().context("open benchmark remem database")?;
    let project = repo_dir.to_string_lossy().to_string();
    let seeded = seed_task_memories(&conn, &project, task)?;
    let seeded_ids = seeded.iter().map(|memory| memory.id).collect::<Vec<_>>();
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
    let mut injected_memory_ids = query_injected_memory_ids(&conn, &project)?;
    injected_memory_ids.extend(seeded_ids.iter().copied());
    injected_memory_ids.sort_unstable();
    injected_memory_ids.dedup();
    let mut output = snapshot.rendered_output;
    append_benchmark_memory_details(&mut output, &seeded_memories);
    let memory_attribution = build_attribution_input(task, &seeded, injected_memory_ids);
    Ok((output, memory_attribution))
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

#[derive(Debug, Clone)]
struct SeededMemoryEvidence {
    id: i64,
    facts: Vec<String>,
}

fn seed_task_memories(
    conn: &rusqlite::Connection,
    project: &str,
    task: &CodingBenchTask,
) -> Result<Vec<SeededMemoryEvidence>> {
    let mut seeded = Vec::new();
    for episode in &task.history_episodes {
        for memory in &episode.memories {
            let saved = save_seed_memory(conn, project, memory)?;
            seeded.push(SeededMemoryEvidence {
                id: saved.id,
                facts: episode.expected_memory_facts.clone(),
            });
        }
    }
    for memory in &task.memories {
        let saved = save_seed_memory(conn, project, memory)?;
        seeded.push(SeededMemoryEvidence {
            id: saved.id,
            facts: task.gold_memory.required_facts.clone(),
        });
    }
    Ok(seeded)
}

fn build_attribution_input(
    task: &CodingBenchTask,
    seeded: &[SeededMemoryEvidence],
    injected_memory_ids: Vec<i64>,
) -> CodingMemoryAttributionInput {
    let mut fact_to_ids: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for memory in seeded {
        for fact in &memory.facts {
            fact_to_ids
                .entry(fact.as_str())
                .or_default()
                .push(memory.id);
        }
    }
    let required = task
        .gold_memory
        .required_facts
        .iter()
        .flat_map(|fact| {
            fact_to_ids
                .get(fact.as_str())
                .into_iter()
                .flatten()
                .copied()
        })
        .collect::<BTreeSet<_>>();
    let forbidden = task
        .gold_memory
        .forbidden_facts
        .iter()
        .flat_map(|fact| {
            fact_to_ids
                .get(fact.as_str())
                .into_iter()
                .flatten()
                .copied()
        })
        .collect::<BTreeSet<_>>();
    CodingMemoryAttributionInput {
        injected_memory_ids,
        relevant_memory_ids: required.into_iter().collect(),
        forbidden_memory_ids: forbidden.into_iter().collect(),
        gold_required_facts: task.gold_memory.required_facts.clone(),
        gold_forbidden_facts: task.gold_memory.forbidden_facts.clone(),
    }
}

fn query_injected_memory_ids(conn: &rusqlite::Connection, project: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT memory_id
         FROM context_injection_items
         WHERE project = ?1
           AND session_id = 'eval-session-start'
           AND status = 'injected'
           AND memory_id IS NOT NULL
         ORDER BY memory_id ASC",
    )?;
    let rows = stmt.query_map([project], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
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
