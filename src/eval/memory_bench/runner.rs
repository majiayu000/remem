use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::eval::bench_artifact::{
    BenchmarkLayer, MemoryCitationEvidence, MemoryDiagnosis, MemoryRetrievalEvidence,
    MemoryRunArtifact, PublicBenchmarkReport, ReportVerifierMetadata, RunEnvironment,
};

use super::baselines::fixture_retrieval_indices;
use super::diagnostics::{
    classify_diagnosis, failure_decomposition, performance_by_condition, performance_metrics,
    score_policy,
};
use super::fixture::load_suite;
use super::types::{
    summarize_by_category, summarize_metrics, summarize_policy, MemoryBenchCondition,
    MemoryBenchEvidence, MemoryBenchRunOutcome, MemoryBenchSuiteFixture, MemoryBenchTask,
    DEFAULT_PUBLIC_ROOT, DEFAULT_REPORT_BENCHMARK_VERSION,
};

const PROJECT: &str = "/tmp/remem-memory-bench/repo";
const READER_PROVIDER: &str = "fixture";
const READER_MODEL: &str = "deterministic-memory-reader";

#[derive(Debug, Clone)]
pub struct MemoryBenchOptions {
    pub suite: String,
    pub condition: Option<String>,
    pub json_out: String,
    pub root: String,
    pub artifact_prefix: Option<String>,
}

pub fn run_memory_bench(options: MemoryBenchOptions) -> Result<PublicBenchmarkReport> {
    let fixture = load_suite(&options.suite)?;
    let conditions = selected_conditions(options.condition.as_deref())?;
    let public_root = PathBuf::from(if options.root.trim().is_empty() {
        DEFAULT_PUBLIC_ROOT
    } else {
        options.root.as_str()
    });
    let json_out = PathBuf::from(&options.json_out);
    let artifact_prefix = options
        .artifact_prefix
        .unwrap_or_else(|| format!("memory/artifacts/{}", fixture.fixture_revision));
    let public_layout = path_starts_with(&json_out, &public_root);
    let artifact_root = if public_layout {
        public_root.join(&artifact_prefix)
    } else {
        sibling_artifact_root(&json_out)
    };
    fs::create_dir_all(&artifact_root).with_context(|| {
        format!(
            "create memory benchmark artifacts {}",
            artifact_root.display()
        )
    })?;

    let mut outcomes = Vec::new();
    let mut run_artifacts = Vec::new();
    for condition in conditions {
        for task in &fixture.tasks {
            let outcome = run_task(&fixture, condition, task)?;
            let run_json_path = write_run_artifacts(
                &fixture,
                &outcome,
                task,
                &artifact_root,
                &public_root,
                public_layout,
            )?;
            run_artifacts.push(run_json_path);
            outcomes.push(outcome);
        }
    }

    let aggregate_metrics = json!({
        "suite": fixture.suite,
        "suite_version": fixture.version,
        "fixture_revision": fixture.fixture_revision,
        "run_count": outcomes.len(),
        "overall": summarize_metrics(&outcomes),
        "by_category": summarize_by_category(&outcomes),
        "conditions": summarize_by_condition(&outcomes),
        "failure_decomposition": failure_decomposition(&outcomes),
        "performance": performance_by_condition(&outcomes),
        "policy": summarize_policy(&outcomes),
    });
    let report = PublicBenchmarkReport {
        schema_version: 1,
        benchmark_id: fixture.benchmark_id.clone(),
        benchmark_version: fixture.version.clone(),
        layer: BenchmarkLayer::MemorySystemCapability,
        conditions: outcomes
            .iter()
            .map(|outcome| outcome.condition.as_str().to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        schema_refs: vec![
            "schemas/benchmark-manifest.schema.json".to_string(),
            "schemas/memory-report.schema.json".to_string(),
            "schemas/memory-run.schema.json".to_string(),
            "schemas/reproduction-metadata.schema.json".to_string(),
        ],
        run_artifacts,
        aggregate_metrics,
        claim_level: "directional_memory_suite_no_public_claim".to_string(),
        verifier: ReportVerifierMetadata {
            required: true,
            schema_version: 1,
        },
    };

    if let Some(parent) = json_out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create memory benchmark report dir {}", parent.display())
            })?;
        }
    }
    fs::write(&json_out, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("write memory benchmark report {}", json_out.display()))?;
    Ok(report)
}

fn selected_conditions(condition: Option<&str>) -> Result<Vec<MemoryBenchCondition>> {
    match condition {
        Some(raw) => {
            let condition = MemoryBenchCondition::parse(raw)
                .with_context(|| format!("unknown memory benchmark condition {raw}"))?;
            Ok(vec![condition])
        }
        None => Ok(MemoryBenchCondition::ALL.to_vec()),
    }
}

fn run_task(
    fixture: &MemoryBenchSuiteFixture,
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
) -> Result<MemoryBenchRunOutcome> {
    let retrieved = if let Some(indices) = fixture_retrieval_indices(condition, task) {
        indices
            .into_iter()
            .map(|idx| RetrievedEvidence::from_fixture(idx, &task.evidence[idx]))
            .collect()
    } else {
        retrieve_with_remem_search(task)?
    };
    Ok(score_task(fixture, condition, task, retrieved))
}

fn retrieve_with_remem_search(task: &MemoryBenchTask) -> Result<Vec<RetrievedEvidence>> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let mut by_memory_id = BTreeMap::new();
    for evidence in task
        .evidence
        .iter()
        .filter(|evidence| evidence.retention_allowed)
    {
        let files = if evidence.files.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&evidence.files)?)
        };
        let id = crate::memory::insert_memory_full_with_reference_time(
            &conn,
            Some(&evidence.event_id),
            PROJECT,
            evidence.topic_key.as_deref(),
            &evidence.title,
            &evidence.content,
            &evidence.memory_type,
            files.as_deref(),
            Some("main"),
            &evidence.scope,
            evidence.created_at_epoch,
            evidence.created_at_epoch,
        )?;
        if evidence.status != "active" {
            conn.execute(
                "UPDATE memories SET status = ?1 WHERE id = ?2",
                rusqlite::params![evidence.status, id],
            )?;
        }
        by_memory_id.insert(id, evidence);
    }

    let hits = crate::retrieval::search::search_with_branch(
        &conn,
        Some(&task.query),
        Some(PROJECT),
        None,
        5,
        0,
        false,
        Some("main"),
    )?;
    Ok(hits
        .into_iter()
        .filter_map(|memory| {
            by_memory_id
                .get(&memory.id)
                .map(|evidence| RetrievedEvidence::from_memory(memory.id, evidence))
        })
        .collect())
}

fn score_task(
    fixture: &MemoryBenchSuiteFixture,
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
    retrieved: Vec<RetrievedEvidence>,
) -> MemoryBenchRunOutcome {
    let gold = task
        .gold_supporting_event_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let forbidden = task
        .forbidden_event_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let retrieved_events = retrieved
        .iter()
        .map(|item| item.event_id.clone())
        .collect::<Vec<_>>();
    let retrieved_set = retrieved_events.iter().cloned().collect::<BTreeSet<_>>();
    let retrieved_gold = gold
        .intersection(&retrieved_set)
        .cloned()
        .collect::<Vec<_>>();
    let missing_event_ids = gold.difference(&retrieved_set).cloned().collect::<Vec<_>>();
    let forbidden_count = forbidden.intersection(&retrieved_set).count();
    let support_coverage = ratio(retrieved_gold.len(), gold.len());
    let evidence_complete = missing_event_ids.is_empty() && forbidden_count == 0;
    let expected_policy_abstention = task
        .policy
        .as_ref()
        .map(|policy| policy.expected_policy_abstention)
        .unwrap_or(false);
    let abstained = expected_policy_abstention || !evidence_complete;
    let answer_score = if evidence_complete
        || ((task.abstention_allowed || expected_policy_abstention) && abstained)
    {
        1.0
    } else {
        0.0
    };
    let answer_text = if abstained {
        "Insufficient benchmark evidence to answer.".to_string()
    } else {
        task.expected_answer.clone()
    };
    let cited_memory_ids = if abstained {
        Vec::new()
    } else {
        retrieved
            .iter()
            .filter(|item| gold.contains(&item.event_id))
            .map(|item| item.memory_id)
            .collect()
    };
    let cited_event_ids = if abstained {
        Vec::new()
    } else {
        retrieved_gold.clone()
    };
    let citation_recall = if abstained {
        0.0
    } else {
        ratio(cited_event_ids.len(), gold.len())
    };
    let citation_precision = if abstained || cited_event_ids.is_empty() {
        0.0
    } else {
        ratio(
            cited_event_ids.len(),
            cited_event_ids.len() + forbidden_count,
        )
    };
    let staleness_accuracy = if forbidden_count == 0 { 1.0 } else { 0.0 };
    let expected_abstention = condition == MemoryBenchCondition::NoMemory
        || task.abstention_allowed
        || expected_policy_abstention;
    let abstention_accuracy = if abstained == expected_abstention {
        1.0
    } else {
        0.0
    };
    let policy = score_policy(condition, task, &retrieved_events, abstained);
    let reader_input = build_reader_input(condition, task, &retrieved);
    let diagnosis =
        classify_diagnosis(condition, task, &missing_event_ids, answer_score, abstained);
    let performance = performance_metrics(condition, task, &reader_input, retrieved.len());
    let retrieved_evidence_json = json!({
        "suite": fixture.suite,
        "fixture_revision": fixture.fixture_revision,
        "condition": condition.as_str(),
        "task_id": task.id,
        "retrieved": retrieved.iter().map(RetrievedEvidence::to_json).collect::<Vec<_>>(),
    });
    let mut diagnosis_notes = Vec::new();
    if !missing_event_ids.is_empty() {
        diagnosis_notes.push(format!(
            "missing supporting evidence: {}",
            missing_event_ids.join(",")
        ));
    }
    if forbidden_count > 0 {
        diagnosis_notes.push(format!(
            "retrieved forbidden evidence count: {forbidden_count}"
        ));
    }
    if policy.policy_failure_count > 0 {
        diagnosis_notes.push(format!(
            "structured policy failure count: {}",
            policy.policy_failure_count
        ));
    }
    if policy.non_retention_leaked {
        diagnosis_notes.push("non-retention leak detected".to_string());
    }
    if policy.false_blocked {
        diagnosis_notes.push("approved policy evidence was falsely blocked".to_string());
    }

    MemoryBenchRunOutcome {
        condition,
        task_id: task.id.clone(),
        category: task.category.clone(),
        run_index: 0,
        retrieved_memory_ids: retrieved.iter().map(|item| item.memory_id).collect(),
        retrieved_event_ids: retrieved_events,
        cited_memory_ids,
        cited_event_ids,
        missing_event_ids,
        answer_text,
        abstained,
        support_coverage,
        answer_score,
        citation_recall,
        citation_precision,
        staleness_accuracy,
        abstention_accuracy,
        forbidden_evidence_count: forbidden_count,
        reader_input,
        retrieved_evidence_json,
        diagnosis_notes,
        policy,
        diagnosis,
        performance,
    }
}

fn build_reader_input(
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
    retrieved: &[RetrievedEvidence],
) -> String {
    let mut input = String::new();
    input.push_str(&format!("condition: {}\n", condition.as_str()));
    input.push_str(&format!("task_id: {}\n", task.id));
    input.push_str(&format!("category: {}\n", task.category));
    input.push_str(&format!(
        "reference_time_epoch: {}\n\n",
        task.reference_time_epoch
    ));
    input.push_str("question:\n");
    input.push_str(&task.prompt);
    input.push_str("\n\nretrieved_evidence:\n");
    if retrieved.is_empty() {
        input.push_str("(none)\n");
    } else {
        for evidence in retrieved {
            input.push_str(&format!(
                "- memory_id={} event_id={} status={} title={}\n  {}\n",
                evidence.memory_id,
                evidence.event_id,
                evidence.status,
                evidence.title,
                evidence.content
            ));
        }
    }
    input
}

#[allow(clippy::too_many_arguments)]
fn write_run_artifacts(
    fixture: &MemoryBenchSuiteFixture,
    outcome: &MemoryBenchRunOutcome,
    task: &MemoryBenchTask,
    artifact_root: &Path,
    public_root: &Path,
    public_layout: bool,
) -> Result<String> {
    let run_dir = artifact_root.join(format!(
        "{}-{}",
        outcome.condition.as_str(),
        outcome.task_id
    ));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("create memory benchmark run dir {}", run_dir.display()))?;

    let reader_input_path = run_dir.join("reader_input.txt");
    let retrieved_path = run_dir.join("retrieved_evidence.json");
    let answer_path = run_dir.join("answer.json");
    let score_path = run_dir.join("score.json");
    let diagnosis_path = run_dir.join("diagnosis.json");
    let snapshot_path = run_dir.join("remem.db.snapshot.tar.zst");
    let run_path = run_dir.join("run.json");

    fs::write(&reader_input_path, &outcome.reader_input)?;
    fs::write(
        &retrieved_path,
        serde_json::to_string_pretty(&outcome.retrieved_evidence_json)?,
    )?;
    fs::write(
        &answer_path,
        serde_json::to_string_pretty(&json!({
            "text": outcome.answer_text,
            "abstained": outcome.abstained,
            "score": outcome.answer_score,
        }))?,
    )?;
    fs::write(
        &score_path,
        serde_json::to_string_pretty(&json!({
            "support_coverage": outcome.support_coverage,
            "answer_score": outcome.answer_score,
            "citation_recall": outcome.citation_recall,
            "citation_precision": outcome.citation_precision,
            "staleness_accuracy": outcome.staleness_accuracy,
            "abstention_accuracy": outcome.abstention_accuracy,
            "forbidden_evidence_count": outcome.forbidden_evidence_count,
        }))?,
    )?;
    fs::write(
        &diagnosis_path,
        serde_json::to_string_pretty(&json!({
            "notes": outcome.diagnosis_notes,
            "missing_event_ids": outcome.missing_event_ids,
        }))?,
    )?;
    fs::write(
        &snapshot_path,
        "fixture placeholder: in-memory sqlite seeded from public suite evidence\n",
    )?;

    let artifacts = BTreeMap::from([
        (
            "reader_input".to_string(),
            artifact_path(&reader_input_path, public_root, public_layout)?,
        ),
        (
            "retrieved_evidence".to_string(),
            artifact_path(&retrieved_path, public_root, public_layout)?,
        ),
        (
            "answer".to_string(),
            artifact_path(&answer_path, public_root, public_layout)?,
        ),
        (
            "score".to_string(),
            artifact_path(&score_path, public_root, public_layout)?,
        ),
        (
            "diagnosis".to_string(),
            artifact_path(&diagnosis_path, public_root, public_layout)?,
        ),
        (
            "remem_db_snapshot".to_string(),
            artifact_path(&snapshot_path, public_root, public_layout)?,
        ),
    ]);
    let run = MemoryRunArtifact {
        schema_version: 1,
        benchmark_version: DEFAULT_REPORT_BENCHMARK_VERSION.to_string(),
        layer: BenchmarkLayer::MemorySystemCapability,
        suite: fixture.suite.clone(),
        condition: outcome.condition.as_str().to_string(),
        task_id: outcome.task_id.clone(),
        run_index: outcome.run_index,
        reference_time_epoch: task.reference_time_epoch,
        reader_model: json!({
            "provider": READER_PROVIDER,
            "model": READER_MODEL,
            "temperature": 0,
            "prompt_hash": prompt_hash(&task.prompt),
        }),
        environment: RunEnvironment {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            remem_commit: current_git_rev().unwrap_or_else(|| "unknown".to_string()),
            remem_data_dir: format!(
                "temp://remem-memory-bench/{}/{}/{}",
                fixture.fixture_revision,
                outcome.condition.as_str(),
                outcome.task_id
            ),
            docker_image_digest: Some("local-fixture-no-docker".to_string()),
            fixture_revision: Some(fixture.fixture_revision.clone()),
            repo_base_commit: None,
        },
        answer: json!({
            "text": outcome.answer_text,
            "abstained": outcome.abstained,
            "score": outcome.answer_score,
            "score_method": "deterministic_fixture",
            "temporal_as_of_correct": outcome.staleness_accuracy == 1.0,
            "no_answer_correct": outcome.abstention_accuracy == 1.0,
        }),
        retrieval: MemoryRetrievalEvidence {
            retrieved_memory_ids: outcome.retrieved_memory_ids.clone(),
            retrieved_supporting_evidence_ids: outcome.retrieved_event_ids.clone(),
            gold_supporting_event_ids: task.gold_supporting_event_ids.clone(),
            missing_supporting_evidence_ids: outcome.missing_event_ids.clone(),
        },
        evidence: MemoryCitationEvidence {
            cited_memory_ids: outcome.cited_memory_ids.clone(),
            cited_event_ids: outcome.cited_event_ids.clone(),
        },
        metrics: json!({
            "ingest_tokens": outcome.performance.ingest_tokens,
            "query_tokens": outcome.performance.query_tokens,
            "reader_tokens": outcome.performance.reader_tokens,
            "retrieval_latency_ms": outcome.performance.retrieval_latency_ms,
            "end_to_end_latency_ms": outcome.performance.end_to_end_latency_ms,
            "rows_written": outcome.performance.rows_written,
            "support_coverage": outcome.support_coverage,
            "answer_score": outcome.answer_score,
            "citation_recall": outcome.citation_recall,
            "citation_precision": outcome.citation_precision,
            "staleness_accuracy": outcome.staleness_accuracy,
            "abstention_accuracy": outcome.abstention_accuracy,
            "forbidden_evidence_count": outcome.forbidden_evidence_count,
            "retrieved_memory_count": outcome.retrieved_memory_ids.len(),
            "policy": {
                "active_claim_count": outcome.policy.active_claim_count,
                "candidate_count": outcome.policy.candidate_count,
                "summary_input_count": outcome.policy.summary_input_count,
                "policy_failure_count": outcome.policy.policy_failure_count,
            },
        }),
        diagnosis: MemoryDiagnosis {
            write_side_gap: outcome.diagnosis.write_side_gap,
            retrieval_side_gap: outcome.diagnosis.retrieval_side_gap,
            reader_gap: outcome.diagnosis.reader_gap,
            policy_abstention: outcome.diagnosis.policy_abstention,
            notes: outcome.diagnosis_notes.clone(),
        },
        artifacts,
    };
    fs::write(&run_path, serde_json::to_string_pretty(&run)?)?;
    artifact_path(&run_path, public_root, public_layout)
}

fn summarize_by_condition(
    outcomes: &[MemoryBenchRunOutcome],
) -> BTreeMap<String, super::types::MemoryBenchMetricSummary> {
    let mut grouped: BTreeMap<String, Vec<&MemoryBenchRunOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped
            .entry(outcome.condition.as_str().to_string())
            .or_default()
            .push(outcome);
    }
    grouped
        .into_iter()
        .map(|(condition, runs)| (condition, summarize_metrics(runs)))
        .collect()
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    path.starts_with(root) || (!path.is_absolute() && root.is_relative() && path.starts_with(root))
}

fn sibling_artifact_root(json_out: &Path) -> PathBuf {
    let stem = json_out
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("remem-memory-bench");
    let dir_name = format!("{stem}-artifacts");
    json_out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(dir_name)
}

fn artifact_path(path: &Path, public_root: &Path, public_layout: bool) -> Result<String> {
    if public_layout {
        let relative = path.strip_prefix(public_root).with_context(|| {
            format!(
                "artifact path {} must be inside public root {}",
                path.display(),
                public_root.display()
            )
        })?;
        Ok(path_to_string(relative))
    } else {
        Ok(path_to_string(path))
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn prompt_hash(prompt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prompt.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn current_git_rev() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[derive(Debug, Clone)]
struct RetrievedEvidence {
    memory_id: i64,
    event_id: String,
    title: String,
    content: String,
    status: String,
    source_anchor: String,
}

impl RetrievedEvidence {
    fn from_fixture(index: usize, evidence: &MemoryBenchEvidence) -> Self {
        Self::from_memory((index + 1) as i64, evidence)
    }

    fn from_memory(memory_id: i64, evidence: &MemoryBenchEvidence) -> Self {
        Self {
            memory_id,
            event_id: evidence.event_id.clone(),
            title: evidence.title.clone(),
            content: evidence.content.clone(),
            status: evidence.status.clone(),
            source_anchor: evidence.source_anchor.clone(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "memory_id": self.memory_id,
            "event_id": self.event_id,
            "title": self.title,
            "content": self.content,
            "status": self.status,
            "source_anchor": self.source_anchor,
        })
    }
}
