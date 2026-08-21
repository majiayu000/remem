use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::types::{
    BenchCondition, CodingBenchFixture, CodingBenchTask, CodingMemoryAttributionInput,
    CuratorLogAttachment, SeedMemory,
};
use super::{RememContextAuditSnapshot, RememContextAuditStatus};

pub(super) const BENCHMARK_CONTEXT_ENV_OVERRIDES: &[&str] = &[
    "REMEM_CIPHER_KEY",
    "REMEM_CONFIG",
    "REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT",
    "REMEM_CONTEXT_CORE_CHAR_LIMIT",
    "REMEM_CONTEXT_CORE_ITEM_LIMIT",
    "REMEM_CONTEXT_DEBUG",
    "REMEM_CONTEXT_LESSON_CHAR_LIMIT",
    "REMEM_CONTEXT_LESSON_LIMIT",
    "REMEM_CONTEXT_MEMORY_INDEX_CHAR_LIMIT",
    "REMEM_CONTEXT_MEMORY_INDEX_LIMIT",
    "REMEM_CONTEXT_OBSERVATIONS",
    "REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT",
    "REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT",
    "REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT",
    "REMEM_CONTEXT_RELEVANCE_K",
    "REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT",
    "REMEM_CONTEXT_SESSION_COUNT",
    "REMEM_CONTEXT_TOTAL_CHAR_LIMIT",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDINGS_API_KEY_ENV",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDINGS_DIMENSIONS",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_HOOK_TIMEOUT_SECS",
    "REMEM_EMBEDDINGS_MODEL",
    "REMEM_EMBEDDINGS_MODEL_DIR",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDINGS_TIMEOUT_SECS",
    "REMEM_EMBEDDING_API_KEY",
    "REMEM_EMBEDDING_BASE_URL",
    "REMEM_EMBEDDING_DIMENSIONS",
    "REMEM_EMBEDDING_MODEL",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_PREF_EMBEDDING_THRESHOLD",
    "REMEM_RERANK_DEADLINE_MS",
    "REMEM_RERANK_ENABLED",
    "REMEM_RERANK_MAX_DOCUMENT_BYTES",
    "REMEM_RERANK_MODEL_DIR",
    "REMEM_RERANK_PRESET",
    "REMEM_RERANK_TOP_K",
    "REMEM_RERANK_TOP_N",
    "REMEM_USAGE_WEIGHT",
];

#[derive(Debug, Clone)]
pub struct ConditionSetup {
    pub env: Vec<(String, String)>,
    pub prompt_note: Option<String>,
    pub memory_attribution: CodingMemoryAttributionInput,
    pub context_audit_status: RememContextAuditStatus,
    pub context_audit_failure_reason: Option<String>,
    pub remem_context_audit: Option<RememContextAuditSnapshot>,
    pub curator_log: Option<CuratorLogAttachment>,
    pub e2e_pipeline: Option<super::e2e::E2ePipelineTrace>,
}

pub async fn apply_condition(
    condition: BenchCondition,
    fixture: &CodingBenchFixture,
    task: &CodingBenchTask,
    repo_dir: &Path,
    data_dir: &Path,
    curator_root: Option<&Path>,
) -> Result<ConditionSetup> {
    match condition {
        BenchCondition::NoMemory => Ok(ConditionSetup {
            env: vec![("REMEM_DISABLE_HOOKS".to_string(), "1".to_string())],
            prompt_note: None,
            memory_attribution: CodingMemoryAttributionInput::default(),
            context_audit_status: RememContextAuditStatus::NotApplicable,
            context_audit_failure_reason: None,
            remem_context_audit: None,
            curator_log: None,
            e2e_pipeline: None,
        }),
        BenchCondition::CuratedFileBudgeted => {
            let curator_root = curator_root.context(
                "curated_file_budgeted requires --curator-root with verified task inputs",
            )?;
            let curator_log =
                super::curator::install_budgeted_memory(curator_root, task, repo_dir)?;
            Ok(ConditionSetup {
                env: vec![("REMEM_DISABLE_HOOKS".to_string(), "1".to_string())],
                prompt_note: Some(
                    "A target-blind, budgeted MEMORY.md file is available in the repo. Read it before editing."
                        .to_string(),
                ),
                memory_attribution: CodingMemoryAttributionInput::default(),
                context_audit_status: RememContextAuditStatus::NotApplicable,
                context_audit_failure_reason: None,
                remem_context_audit: None,
                curator_log: Some(curator_log),
                e2e_pipeline: None,
            })
        }
        BenchCondition::CuratedFileExpert => {
            let content = task
                .curated_context
                .as_deref()
                .or(fixture.curated_context.as_deref())
                .context(
                    "curated_file_expert condition requires curated_context in fixture or task",
                )?;
            fs::write(repo_dir.join("MEMORY.md"), content).context("write curated MEMORY.md")?;
            Ok(ConditionSetup {
                env: vec![("REMEM_DISABLE_HOOKS".to_string(), "1".to_string())],
                prompt_note: Some(
                    "A curated MEMORY.md file is available in the repo. Read it before editing."
                        .to_string(),
                ),
                memory_attribution: CodingMemoryAttributionInput::default(),
                context_audit_status: RememContextAuditStatus::NotApplicable,
                context_audit_failure_reason: None,
                remem_context_audit: None,
                curator_log: None,
                e2e_pipeline: None,
            })
        }
        BenchCondition::RememSeededSessionStart => {
            let (rendered, memory_attribution, audit_contract) =
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
                    "The exact audited remem SessionStart context is available at REMEM_CONTEXT.md. Read it before editing."
                        .to_string(),
                ),
                memory_attribution,
                context_audit_status: audit_contract.status,
                context_audit_failure_reason: audit_contract.failure_reason,
                remem_context_audit: audit_contract.snapshot,
                curator_log: None,
                e2e_pipeline: None,
            })
        }
        BenchCondition::RememE2e => {
            let memory_config = curator_root
                .context("internal remem_e2e setup requires an explicit memory config path")?;
            let prepared = super::e2e::prepare_remem_e2e(data_dir, task, memory_config).await?;
            fs::write(
                repo_dir.join("REMEM_CONTEXT.md"),
                &prepared.rendered_context,
            )
            .context("write production-derived remem_e2e SessionStart context")?;
            Ok(ConditionSetup {
                env: vec![
                    (
                        "REMEM_DATA_DIR".to_string(),
                        data_dir.to_string_lossy().to_string(),
                    ),
                    ("REMEM_ALLOW_PLAINTEXT_DB".to_string(), "1".to_string()),
                ],
                prompt_note: Some(
                    "The exact audited remem_e2e SessionStart context is available at REMEM_CONTEXT.md. Read it before editing."
                        .to_string(),
                ),
                memory_attribution: prepared.memory_attribution,
                context_audit_status: RememContextAuditStatus::Verified,
                context_audit_failure_reason: None,
                remem_context_audit: Some(prepared.context_audit),
                curator_log: None,
                e2e_pipeline: Some(prepared.trace),
            })
        }
        BenchCondition::OracleEvidence
        | BenchCondition::RememOracleRetrieval
        | BenchCondition::FullHistory
        | BenchCondition::RememNoEnrichment
        | BenchCondition::RememFtsOnly => {
            bail!(
                "{} live execution is not implemented; use --dry-run or select remem_seeded_sessionstart/curated_file_expert for implemented diagnostics",
                condition.as_str()
            )
        }
    }
}

struct RememAuditContract {
    status: RememContextAuditStatus,
    failure_reason: Option<String>,
    snapshot: Option<RememContextAuditSnapshot>,
}

fn render_seeded_remem_context(
    data_dir: &Path,
    repo_dir: &Path,
    task: &CodingBenchTask,
) -> Result<(String, CodingMemoryAttributionInput, RememAuditContract)> {
    let _env_lock = crate::runtime_config::ENV_LOCK.lock().map_err(|error| {
        anyhow::anyhow!("acquire benchmark environment lock before rendering context: {error}")
    })?;
    render_seeded_remem_context_while_env_locked(data_dir, repo_dir, task)
}

fn render_seeded_remem_context_while_env_locked(
    data_dir: &Path,
    repo_dir: &Path,
    task: &CodingBenchTask,
) -> Result<(String, CodingMemoryAttributionInput, RememAuditContract)> {
    fs::create_dir_all(data_dir).context("create benchmark REMEM_DATA_DIR")?;
    let _env = ScopedEnvVars::set_many([
        ("REMEM_DATA_DIR", data_dir.as_os_str().to_os_string()),
        ("REMEM_ALLOW_PLAINTEXT_DB", OsString::from("1")),
        ("REMEM_CONTEXT_BUNDLE_RENDER_MODE", OsString::from("bundle")),
        ("REMEM_CONTEXT_GATE_HOSTS", OsString::from("codex-cli")),
    ]);
    let _context_env = ScopedEnvVars::remove_many(BENCHMARK_CONTEXT_ENV_OVERRIDES);
    let conn = crate::db::open_db().context("open benchmark remem database")?;
    let project = repo_dir.to_string_lossy().to_string();
    let seeded = seed_task_memories(&conn, &project, task)?;
    let emission =
        crate::context::session_start_benchmark_emission(&project, &project, "codex-cli")
            .context("render and persist benchmark SessionStart context")?;
    let audit_contract = match emission.injection_run_id.as_deref() {
        Some(injection_run_id) => {
            match super::audit_contract::load_context_audit_snapshot(&conn, injection_run_id) {
                Ok(Some(snapshot)) => {
                    match super::audit_contract::verify_snapshot_against_persisted_injection(
                        &conn,
                        &snapshot,
                        &emission.rendered_output,
                    ) {
                        Ok(()) => RememAuditContract {
                            status: RememContextAuditStatus::Verified,
                            failure_reason: None,
                            snapshot: Some(snapshot),
                        },
                        Err(error) => RememAuditContract {
                            status: RememContextAuditStatus::ContractFailure,
                            failure_reason: Some(format!(
                                "ContextAudit differs from persisted injection_run_id={injection_run_id}: {error}"
                            )),
                            snapshot: None,
                        },
                    }
                }
                Ok(None) => RememAuditContract {
                    status: RememContextAuditStatus::ContractFailure,
                    failure_reason: Some(format!(
                        "missing ContextAudit for injection_run_id={injection_run_id}"
                    )),
                    snapshot: None,
                },
                Err(error) => RememAuditContract {
                    status: RememContextAuditStatus::ContractFailure,
                    failure_reason: Some(format!(
                        "invalid ContextAudit for injection_run_id={injection_run_id}: {error}"
                    )),
                    snapshot: None,
                },
            }
        }
        None => RememAuditContract {
            status: RememContextAuditStatus::ContractFailure,
            failure_reason: Some(
                "SessionStart emitted remem context without a persisted ContextAudit".to_string(),
            ),
            snapshot: None,
        },
    };
    let injected_memory_ids =
        query_injected_memory_ids(&conn, emission.injection_run_id.as_deref())?;
    let memory_attribution = build_attribution_input(task, &seeded, injected_memory_ids);
    Ok((emission.rendered_output, memory_attribution, audit_contract))
}

fn save_seed_memory(
    conn: &rusqlite::Connection,
    project: &str,
    memory: &SeedMemory,
) -> Result<crate::memory::service::SaveMemoryResult> {
    crate::memory::service::save_memory_for_benchmark_fixture(
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

fn query_injected_memory_ids(
    conn: &rusqlite::Connection,
    injection_run_id: Option<&str>,
) -> Result<Vec<i64>> {
    let Some(injection_run_id) = injection_run_id else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT DISTINCT memory_id
         FROM context_injection_items
         WHERE injection_run_id = ?1
           AND status = 'injected'
           AND memory_id IS NOT NULL
         ORDER BY memory_id ASC",
    )?;
    let rows = stmt.query_map([injection_run_id], |row| row.get::<_, i64>(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

pub(super) struct ScopedEnvVars {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ScopedEnvVars {
    pub(super) fn set_many<const N: usize>(values: [(&'static str, OsString); N]) -> Self {
        let previous = values
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for (key, value) in values {
            std::env::set_var(key, value);
        }
        Self { previous }
    }

    pub(super) fn remove_many(keys: &[&'static str]) -> Self {
        let previous = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for key in keys {
            std::env::remove_var(key);
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

    #[tokio::test]
    async fn control_conditions_mark_context_audit_not_applicable() -> Result<()> {
        let fixture = super::super::fixture::load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let task = fixture.tasks.first().context("missing coding-bench task")?;
        let root = crate::db::test_support::ScopedTestDataDir::new("coding-bench-controls");
        let repo_dir = root.path.join("repo");
        let data_dir = root.path.join("remem-data");
        fs::create_dir_all(&repo_dir)?;

        for condition in [BenchCondition::NoMemory, BenchCondition::CuratedFileExpert] {
            let setup =
                apply_condition(condition, &fixture, task, &repo_dir, &data_dir, None).await?;
            assert_eq!(
                setup.context_audit_status,
                RememContextAuditStatus::NotApplicable
            );
            assert!(setup.context_audit_failure_reason.is_none());
            assert!(setup.remem_context_audit.is_none());
        }
        Ok(())
    }

    #[test]
    fn remem_condition_reads_verified_production_sessionstart_audit() -> Result<()> {
        let fixture = super::super::fixture::load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let task = fixture.tasks.first().context("missing coding-bench task")?;
        let root = crate::db::test_support::ScopedTestDataDir::new("coding-bench-audit");
        let repo_dir = root.path.join("repo");
        let data_dir = root.path.join("remem-data");
        fs::create_dir_all(&repo_dir)?;
        let _env_lock = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .map_err(|error| anyhow::anyhow!("acquire test environment lock: {error}"))?;
        let _host_policy = ScopedEnvVars::set_many([
            ("REMEM_CONTEXT_TOTAL_CHAR_LIMIT", OsString::from("1")),
            ("REMEM_CONTEXT_RELEVANCE_K", OsString::from("0")),
            ("REMEM_RERANK_ENABLED", OsString::from("true")),
            ("REMEM_CONTEXT_GATE_HOSTS", OsString::from("claude-code")),
        ]);

        let (output, attribution, contract) =
            render_seeded_remem_context_while_env_locked(&data_dir, &repo_dir, task)?;
        assert!(output.chars().count() > 1_000, "{output}");
        assert!(!output.contains("truncated to REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
        assert_eq!(
            std::env::var("REMEM_CONTEXT_TOTAL_CHAR_LIMIT").as_deref(),
            Ok("1")
        );
        assert_eq!(
            std::env::var("REMEM_CONTEXT_GATE_HOSTS").as_deref(),
            Ok("claude-code")
        );
        assert!(!output.contains("## Benchmark Memory Details"));
        assert_eq!(
            contract.status,
            RememContextAuditStatus::Verified,
            "{:?}",
            contract.failure_reason
        );
        assert!(contract.failure_reason.is_none());
        let snapshot = contract
            .snapshot
            .context("missing verified audit snapshot")?;
        super::super::verify_context_audit_snapshot(&snapshot)?;
        let _snapshot_env = ScopedEnvVars::set_many([
            ("REMEM_DATA_DIR", data_dir.as_os_str().to_os_string()),
            ("REMEM_ALLOW_PLAINTEXT_DB", OsString::from("1")),
        ]);
        let snapshot_conn = crate::db::open_db()?;
        super::super::audit_contract::verify_snapshot_against_persisted_injection(
            &snapshot_conn,
            &snapshot,
            &output,
        )?;
        assert!(
            super::super::audit_contract::verify_snapshot_against_persisted_injection(
                &snapshot_conn,
                &snapshot,
                &format!("{output}\ntampered"),
            )
            .unwrap_err()
            .to_string()
            .contains("differs from persisted")
        );
        snapshot_conn.execute(
            "UPDATE context_injection_items
             SET channel = 'tampered'
             WHERE id = (
               SELECT id FROM context_injection_items
               WHERE injection_run_id = ?1 AND item_kind = 'memory'
               ORDER BY id LIMIT 1
             )",
            [&snapshot.injection_run_id],
        )?;
        assert!(
            super::super::audit_contract::verify_snapshot_against_persisted_injection(
                &snapshot_conn,
                &snapshot,
                &output,
            )
            .unwrap_err()
            .to_string()
            .contains("differs from canonical audit")
        );
        assert!(!snapshot.injection_run_id.is_empty());
        assert_eq!(
            snapshot.plan_schema_version,
            crate::retrieval_router::RETRIEVAL_PLAN_SCHEMA_VERSION
        );
        let (audit, _) = crate::context_bundle::persistence::decode_verified_context_audit_json(
            &snapshot.canonical_audit_json,
            snapshot.plan_schema_version,
        )?;
        let mut audited_memory_ids = audit
            .entries
            .iter()
            .filter(|entry| entry.selected)
            .filter_map(|entry| entry.stable_key.strip_prefix("memory:"))
            .map(str::parse::<i64>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        audited_memory_ids.sort_unstable();
        assert_eq!(attribution.injected_memory_ids, audited_memory_ids);
        for memory in task.seed_memories() {
            assert!(!snapshot.canonical_audit_json.contains(&memory.title));
            assert!(!snapshot.canonical_audit_json.contains(&memory.text));
        }
        Ok(())
    }

    #[test]
    fn remem_condition_ignores_and_restores_ambient_cipher_keys() -> Result<()> {
        let fixture = super::super::fixture::load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let task = fixture.tasks.first().context("missing coding-bench task")?;

        for (label, ambient_key) in [
            ("malformed", OsString::from("v2:not-a-valid-raw-key")),
            ("valid", OsString::from(format!("v2:{}", "a".repeat(64)))),
        ] {
            let root = crate::db::test_support::ScopedTestDataDir::new(&format!(
                "coding-bench-ambient-cipher-{label}"
            ));
            let repo_dir = root.path.join("repo");
            let data_dir = root.path.join("remem-data");
            fs::create_dir_all(&repo_dir)?;
            let _env_lock = crate::runtime_config::TEST_ENV_LOCK
                .lock()
                .map_err(|error| anyhow::anyhow!("acquire test environment lock: {error}"))?;
            let _ambient_cipher =
                ScopedEnvVars::set_many([("REMEM_CIPHER_KEY", ambient_key.clone())]);

            let (output, _, contract) =
                render_seeded_remem_context_while_env_locked(&data_dir, &repo_dir, task)?;

            assert!(!output.is_empty());
            assert_eq!(
                contract.status,
                RememContextAuditStatus::Verified,
                "{label}: {:?}",
                contract.failure_reason
            );
            assert_eq!(std::env::var_os("REMEM_CIPHER_KEY"), Some(ambient_key));

            let raw_conn = rusqlite::Connection::open(data_dir.join("remem.db"))?;
            let schema_entries: i64 =
                raw_conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
            assert!(schema_entries > 0, "{label}: benchmark DB has no schema");
        }
        Ok(())
    }
}
