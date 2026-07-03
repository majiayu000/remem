use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::eval::golden::{self, GoldenDataset, GoldenMemory, QueryEvaluation, QueryMetrics};
use crate::memory::Memory;
use crate::retrieval::search::SearchExplain;

pub const DEFAULT_DATASET_PATH: &str = "eval/golden.json";
const REPORT_VERSION: &str = "2026-07-03";
const RANK_K: usize = 10;
const TRACKED_CHANNELS: [&str; 6] = [
    "fts",
    "entity",
    "fact",
    "temporal",
    "vector",
    "like_fallback",
];

#[derive(Debug, Clone)]
pub struct CapacityEvalOptions {
    pub dataset_path: String,
    pub seed: u64,
    pub scales: Vec<usize>,
    pub k: usize,
}

impl Default for CapacityEvalOptions {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            seed: 42,
            scales: vec![1, 10],
            k: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapacityEvalReport {
    pub version: &'static str,
    pub dataset_path: String,
    pub seed: u64,
    pub k: usize,
    pub scale_factors: Vec<usize>,
    pub base_corpus_size: usize,
    pub scales: Vec<CapacityScaleReport>,
    pub degradation: CapacityDegradation,
    pub omitted_followups: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapacityScaleReport {
    pub scale: usize,
    pub corpus_size: usize,
    pub noise_count: usize,
    pub corpus_hash: String,
    pub fused: CapacityFusedMetrics,
    pub channels: BTreeMap<String, CapacityFusedMetrics>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapacityFusedMetrics {
    pub scored_queries: usize,
    pub hit_at_k: f64,
    pub recall_at_k: f64,
    pub ndcg_at_10: f64,
    pub evidence_recall_at_k: f64,
    pub p95_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapacityDegradation {
    pub largest_scale: usize,
    pub fused_recall_at_k_loss: f64,
    pub fused_ndcg_at_10_loss: f64,
    pub fused_evidence_recall_at_k_loss: f64,
    pub channels: BTreeMap<String, CapacityChannelDegradation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapacityChannelDegradation {
    pub recall_at_k_loss: f64,
    pub ndcg_at_10_loss: f64,
    pub evidence_recall_at_k_loss: f64,
}

#[derive(Debug, Clone)]
pub(in crate::eval) struct ScaledDataset {
    pub dataset: GoldenDataset,
    pub noise_count: usize,
    pub corpus_hash: String,
}

pub fn run_capacity_eval(options: CapacityEvalOptions) -> Result<CapacityEvalReport> {
    let dataset = golden::load_dataset(&options.dataset_path)?;
    run_capacity_eval_for_dataset(options, dataset)
}

pub(in crate::eval) fn run_capacity_eval_for_dataset(
    options: CapacityEvalOptions,
    dataset: GoldenDataset,
) -> Result<CapacityEvalReport> {
    validate_capacity_dataset(&dataset)?;
    let scales = normalize_scales(options.scales)?;
    let k = options.k.max(1);
    let mut scale_reports = Vec::with_capacity(scales.len());

    for scale in &scales {
        let scaled = synthesize_capacity_dataset(&dataset, options.seed, *scale)?;
        scale_reports.push(
            evaluate_capacity_scale(*scale, scaled, k)
                .with_context(|| format!("run capacity eval scale {scale}"))?,
        );
    }

    let baseline = scale_reports
        .iter()
        .find(|report| report.scale == 1)
        .context("capacity eval requires a 1x baseline scale")?;
    let largest = scale_reports
        .iter()
        .max_by_key(|report| report.scale)
        .context("capacity eval requires at least one scale")?;
    let degradation = CapacityDegradation {
        largest_scale: largest.scale,
        fused_recall_at_k_loss: positive_loss(
            baseline.fused.recall_at_k,
            largest.fused.recall_at_k,
        ),
        fused_ndcg_at_10_loss: positive_loss(baseline.fused.ndcg_at_10, largest.fused.ndcg_at_10),
        fused_evidence_recall_at_k_loss: positive_loss(
            baseline.fused.evidence_recall_at_k,
            largest.fused.evidence_recall_at_k,
        ),
        channels: channel_degradation(&baseline.channels, &largest.channels),
    };

    Ok(CapacityEvalReport {
        version: REPORT_VERSION,
        dataset_path: options.dataset_path,
        seed: options.seed,
        k,
        scale_factors: scales,
        base_corpus_size: dataset.corpus.len(),
        scales: scale_reports,
        degradation,
        omitted_followups: vec!["nightly_dashboard_ingestion", "50x_nightly_scale"],
    })
}

fn evaluate_capacity_scale(
    scale: usize,
    scaled: ScaledDataset,
    k: usize,
) -> Result<CapacityScaleReport> {
    let conn = Connection::open_in_memory().context("open in-memory capacity eval DB")?;
    crate::migrate::run_migrations(&conn).context("migrate in-memory capacity eval DB")?;
    golden::run::seed_fixture_corpus(&conn, &scaled.dataset.corpus)?;

    let mut fused = MetricAccumulator::default();
    let mut channels = TRACKED_CHANNELS
        .iter()
        .map(|channel| ((*channel).to_string(), MetricAccumulator::default()))
        .collect::<BTreeMap<_, _>>();
    let fetch_limit = k.max(RANK_K) as i64;

    for query in &scaled.dataset.queries {
        let query_tokens = golden::run::estimate_query_tokens(&query.query);
        let started = Instant::now();
        let (results, explain) = crate::retrieval::search::search_with_branch_explain(
            &conn,
            Some(&query.query),
            query.project.as_deref(),
            query.memory_type.as_deref(),
            fetch_limit,
            0,
            false,
            query.branch.as_deref(),
        )?;
        let retrieval_latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let fused_evaluation =
            golden::run::evaluate_query(query, &results, k, query_tokens, retrieval_latency_ms);
        fused.add(&fused_evaluation);

        let channel_hit_ids = channel_hit_ids(explain.as_ref());
        for channel in TRACKED_CHANNELS {
            let ids = channel_hit_ids.get(channel).cloned().unwrap_or_default();
            let memories = load_ordered_memories(&conn, &ids)?;
            let latency_ms = channel_latency_ms(explain.as_ref(), channel);
            let channel_evaluation =
                golden::run::evaluate_query(query, &memories, k, query_tokens, latency_ms);
            channels
                .entry(channel.to_string())
                .or_default()
                .add(&channel_evaluation);
        }
    }

    Ok(CapacityScaleReport {
        scale,
        corpus_size: scaled.dataset.corpus.len(),
        noise_count: scaled.noise_count,
        corpus_hash: scaled.corpus_hash,
        fused: fused.finish(),
        channels: channels
            .into_iter()
            .map(|(channel, accumulator)| (channel, accumulator.finish()))
            .collect(),
    })
}

pub(in crate::eval) fn synthesize_capacity_dataset(
    base: &GoldenDataset,
    seed: u64,
    scale: usize,
) -> Result<ScaledDataset> {
    validate_capacity_dataset(base)?;
    ensure!(scale >= 1, "capacity scale must be >= 1");
    let base_count = base.corpus.len();
    let target_count = base_count
        .checked_mul(scale)
        .context("capacity scale overflows corpus size")?;
    let noise_count = target_count.saturating_sub(base_count);
    let mut dataset = base.clone();
    let mut existing_topic_keys = topic_keys(&dataset.corpus);
    let projects = corpus_projects(&dataset.corpus);

    for noise_index in 0..noise_count {
        let memory = noise_memory(seed, scale, noise_index, &projects, &existing_topic_keys)?;
        if let Some(topic_key) = memory.topic_key.as_ref() {
            existing_topic_keys.insert(topic_key.clone());
        }
        dataset.corpus.push(memory);
    }

    let corpus_hash = corpus_hash(&dataset.corpus)?;
    Ok(ScaledDataset {
        dataset,
        noise_count,
        corpus_hash,
    })
}

pub(crate) fn normalize_scales(scales: Vec<usize>) -> Result<Vec<usize>> {
    ensure!(
        !scales.is_empty(),
        "capacity eval requires at least one scale"
    );
    let mut normalized = BTreeSet::new();
    for scale in scales {
        ensure!(scale >= 1, "capacity scale must be >= 1");
        normalized.insert(scale);
    }
    ensure!(
        normalized.contains(&1),
        "capacity eval requires scale 1 as the baseline"
    );
    Ok(normalized.into_iter().collect())
}

fn validate_capacity_dataset(dataset: &GoldenDataset) -> Result<()> {
    ensure!(
        dataset.has_fixture_corpus(),
        "capacity eval requires a golden dataset with a fixture corpus"
    );
    ensure!(
        !dataset.queries.is_empty(),
        "capacity eval requires at least one query"
    );
    Ok(())
}

fn positive_loss(baseline: f64, current: f64) -> f64 {
    (baseline - current).max(0.0)
}

#[derive(Default)]
struct MetricAccumulator {
    scored_queries: usize,
    hit_at_k: f64,
    recall_at_k: f64,
    ndcg_at_10: f64,
    evidence_recall_at_k: f64,
    latencies_ms: Vec<f64>,
}

impl MetricAccumulator {
    fn add(&mut self, evaluation: &QueryEvaluation) {
        self.latencies_ms.push(evaluation.retrieval_latency_ms);
        if let Some(metrics) = evaluation.metrics.as_ref() {
            self.add_metrics(metrics);
        }
    }

    fn add_metrics(&mut self, metrics: &QueryMetrics) {
        self.scored_queries += 1;
        self.hit_at_k += metrics.hit_at_k;
        self.recall_at_k += metrics.recall_at_k;
        self.ndcg_at_10 += metrics.ndcg_at_10;
        self.evidence_recall_at_k += metrics.evidence_recall_at_k;
    }

    fn finish(self) -> CapacityFusedMetrics {
        let denominator = self.scored_queries as f64;
        CapacityFusedMetrics {
            scored_queries: self.scored_queries,
            hit_at_k: mean(self.hit_at_k, denominator),
            recall_at_k: mean(self.recall_at_k, denominator),
            ndcg_at_10: mean(self.ndcg_at_10, denominator),
            evidence_recall_at_k: mean(self.evidence_recall_at_k, denominator),
            p95_latency_ms: positive_zero(golden::run::percentile(self.latencies_ms, 95.0)),
        }
    }
}

fn mean(sum: f64, denominator: f64) -> f64 {
    if denominator == 0.0 {
        0.0
    } else {
        sum / denominator
    }
}

fn positive_zero(value: f64) -> f64 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

fn channel_hit_ids(explain: Option<&SearchExplain>) -> BTreeMap<String, Vec<i64>> {
    explain
        .map(|explain| {
            explain
                .channels
                .iter()
                .filter(|channel| channel.enabled)
                .map(|channel| {
                    (
                        channel.name.clone(),
                        channel
                            .hits
                            .iter()
                            .map(|hit| hit.memory_id)
                            .collect::<Vec<_>>(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn channel_latency_ms(explain: Option<&SearchExplain>, channel: &str) -> f64 {
    positive_zero(
        explain
            .map(|explain| {
                explain
                    .timings
                    .iter()
                    .filter(|timing| timing.phase == channel)
                    .map(|timing| timing.elapsed_ms as f64)
                    .sum()
            })
            .unwrap_or_default(),
    )
}

fn load_ordered_memories(conn: &Connection, ids: &[i64]) -> Result<Vec<Memory>> {
    let loaded = crate::memory::get_memories_by_ids_with_suppressed_policy(conn, ids, None, false)?;
    let id_to_memory = loaded
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect::<HashMap<_, _>>();
    Ok(ids
        .iter()
        .filter_map(|id| id_to_memory.get(id).cloned())
        .collect())
}

fn channel_degradation(
    baseline: &BTreeMap<String, CapacityFusedMetrics>,
    largest: &BTreeMap<String, CapacityFusedMetrics>,
) -> BTreeMap<String, CapacityChannelDegradation> {
    baseline
        .iter()
        .filter_map(|(channel, baseline_metrics)| {
            let largest_metrics = largest.get(channel)?;
            Some((
                channel.clone(),
                CapacityChannelDegradation {
                    recall_at_k_loss: positive_loss(
                        baseline_metrics.recall_at_k,
                        largest_metrics.recall_at_k,
                    ),
                    ndcg_at_10_loss: positive_loss(
                        baseline_metrics.ndcg_at_10,
                        largest_metrics.ndcg_at_10,
                    ),
                    evidence_recall_at_k_loss: positive_loss(
                        baseline_metrics.evidence_recall_at_k,
                        largest_metrics.evidence_recall_at_k,
                    ),
                },
            ))
        })
        .collect()
}

fn topic_keys(corpus: &[GoldenMemory]) -> HashSet<String> {
    corpus
        .iter()
        .filter_map(|memory| memory.topic_key.clone())
        .collect()
}

fn corpus_projects(corpus: &[GoldenMemory]) -> Vec<String> {
    let mut projects = Vec::new();
    let mut seen = HashSet::new();
    for memory in corpus {
        if seen.insert(memory.project.as_str()) {
            projects.push(memory.project.clone());
        }
    }
    projects
}

fn noise_memory(
    seed: u64,
    scale: usize,
    noise_index: usize,
    projects: &[String],
    existing_topic_keys: &HashSet<String>,
) -> Result<GoldenMemory> {
    let project = projects
        .get(slot(seed, noise_index, 0, projects.len()))
        .context("capacity noise requires at least one project")?
        .clone();
    let memory_type = MEMORY_TYPES[noise_index % MEMORY_TYPES.len()].to_string();
    let file_path = FILE_PATHS[slot(seed, noise_index, 1, FILE_PATHS.len())];
    let crate_name = CRATE_NAMES[slot(seed, noise_index, 2, CRATE_NAMES.len())];
    let error = ERROR_SIGNATURES[slot(seed, noise_index, 3, ERROR_SIGNATURES.len())];
    let command = COMMANDS[slot(seed, noise_index, 4, COMMANDS.len())];
    let owner = OWNERS[slot(seed, noise_index, 5, OWNERS.len())];
    let suffix = splitmix64(seed ^ ((scale as u64) << 32) ^ noise_index as u64);
    let topic_key = format!("capacity-noise-{seed:x}-{scale}-{noise_index}-{suffix:x}");
    if existing_topic_keys.contains(&topic_key) {
        bail!("capacity noise topic key collision: {topic_key}");
    }

    Ok(GoldenMemory {
        project,
        topic_key: Some(topic_key),
        title: format!("Capacity noise {crate_name} {error}"),
        content: format!(
            "Capacity distractor {noise_index}: {owner} investigated {file_path} in crate {crate_name}. \
             Command `{command}` returned {error}. The follow-up marker is synthetic-noise-{seed:x}-{suffix:x}."
        ),
        memory_type,
        branch: Some("main".to_string()),
        scope: "project".to_string(),
        status: "active".to_string(),
        files: Some(file_path.to_string()),
        created_at_epoch: Some(1_800_000_000 + noise_index as i64),
        access_count: Some(0),
        last_accessed_epoch: None,
    })
}

fn slot(seed: u64, index: usize, salt: u64, len: usize) -> usize {
    debug_assert!(len > 0);
    let mixed = splitmix64(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ index as u64);
    (mixed as usize) % len
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn corpus_hash(corpus: &[GoldenMemory]) -> Result<String> {
    let bytes = serde_json::to_vec(corpus).context("serialize capacity corpus for hash")?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

impl Display for CapacityEvalReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(
            f,
            "remem eval-capacity - seed={} k={} scales={:?}",
            self.seed, self.k, self.scale_factors
        )?;
        writeln!(f, "dataset: {}", self.dataset_path)?;
        for scale in &self.scales {
            writeln!(
                f,
                "  {}x: corpus={} noise={} R@{}={:.3} nDCG@10={:.3} evidence@{}={:.3} p95={:.2}ms hash={}",
                scale.scale,
                scale.corpus_size,
                scale.noise_count,
                self.k,
                scale.fused.recall_at_k,
                scale.fused.ndcg_at_10,
                self.k,
                scale.fused.evidence_recall_at_k,
                scale.fused.p95_latency_ms,
                scale.corpus_hash
            )?;
            for (channel, metrics) in &scale.channels {
                writeln!(
                    f,
                    "    {channel}: R@{}={:.3} nDCG@10={:.3} evidence@{}={:.3} p95={:.2}ms",
                    self.k,
                    metrics.recall_at_k,
                    metrics.ndcg_at_10,
                    self.k,
                    metrics.evidence_recall_at_k,
                    metrics.p95_latency_ms
                )?;
            }
        }
        writeln!(
            f,
            "degradation at {}x: R@{} loss={:.3}, nDCG@10 loss={:.3}, evidence@{} loss={:.3}",
            self.degradation.largest_scale,
            self.k,
            self.degradation.fused_recall_at_k_loss,
            self.degradation.fused_ndcg_at_10_loss,
            self.k,
            self.degradation.fused_evidence_recall_at_k_loss
        )
    }
}

const MEMORY_TYPES: [&str; 4] = ["decision", "bugfix", "discovery", "lesson"];
const FILE_PATHS: [&str; 8] = [
    "src/noise/cache_guard.rs",
    "src/noise/retry_plan.rs",
    "crates/noise_runtime/src/lib.rs",
    "apps/noise_panel/src/main.ts",
    "tests/noise_contract.rs",
    "docs/noise/runbook.md",
    "src/noise/vector_probe.rs",
    "src/noise/ledger_sink.rs",
];
const CRATE_NAMES: [&str; 8] = [
    "aurora-cache",
    "brass-ledger",
    "cipher-ridge",
    "drift-panel",
    "ember-index",
    "frost-runner",
    "garnet-store",
    "harbor-signal",
];
const ERROR_SIGNATURES: [&str; 8] = [
    "E_CAPACITY_001",
    "E_CAPACITY_017",
    "E_CAPACITY_029",
    "E_CAPACITY_041",
    "E_CAPACITY_053",
    "E_CAPACITY_067",
    "E_CAPACITY_079",
    "E_CAPACITY_083",
];
const COMMANDS: [&str; 6] = [
    "cargo test noise_contract",
    "cargo run -- noise-probe",
    "node --test noise-runtime.test.js",
    "python3 scripts/noise_check.py",
    "cargo clippy --package noise-runtime",
    "remem eval --dataset eval/noise.json",
];
const OWNERS: [&str; 6] = [
    "Ari Vale",
    "Bea Stone",
    "Cato Reed",
    "Dina Moss",
    "Eli Park",
    "Faye Holt",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::golden::{EvidenceRef, GoldenQuery};

    #[test]
    fn capacity_synthesis_is_deterministic_for_same_seed_and_scale() -> Result<()> {
        let dataset = tiny_dataset();
        let first = synthesize_capacity_dataset(&dataset, 7, 3)?;
        let second = synthesize_capacity_dataset(&dataset, 7, 3)?;
        let different_seed = synthesize_capacity_dataset(&dataset, 8, 3)?;

        assert_eq!(first.corpus_hash, second.corpus_hash);
        assert_eq!(first.noise_count, 4);
        assert_eq!(first.dataset.corpus.len(), 6);
        assert_eq!(first.dataset.queries[0].id, dataset.queries[0].id);
        assert_ne!(first.corpus_hash, different_seed.corpus_hash);
        Ok(())
    }

    #[test]
    fn capacity_report_includes_scale_curve_and_degradation() -> Result<()> {
        let report = run_capacity_eval_for_dataset(
            CapacityEvalOptions {
                dataset_path: "inline-test".to_string(),
                seed: 9,
                scales: vec![3, 1],
                k: 5,
            },
            tiny_dataset(),
        )?;

        assert_eq!(report.scale_factors, vec![1, 3]);
        assert_eq!(report.base_corpus_size, 2);
        assert_eq!(report.scales.len(), 2);
        assert_eq!(report.scales[0].scale, 1);
        assert_eq!(report.scales[0].noise_count, 0);
        assert_eq!(report.scales[1].scale, 3);
        assert_eq!(report.scales[1].noise_count, 4);
        assert_eq!(report.degradation.largest_scale, 3);
        assert!(report.scales[0].channels.contains_key("fts"));
        assert!(report.scales[0].channels.contains_key("vector"));
        assert!(report.degradation.channels.contains_key("fts"));

        let json = serde_json::to_value(&report)?;
        assert_eq!(json["seed"], 9);
        assert_eq!(json["scales"][1]["fused"]["scored_queries"], 2);
        assert_eq!(json["scales"][1]["channels"]["fts"]["scored_queries"], 2);
        assert!(json["degradation"]["channels"]["fts"]["recall_at_k_loss"].is_number());
        assert!(json["omitted_followups"]
            .as_array()
            .expect("omitted followups should be an array")
            .contains(&serde_json::json!("nightly_dashboard_ingestion")));
        assert!(!json["omitted_followups"]
            .as_array()
            .expect("omitted followups should be an array")
            .contains(&serde_json::json!("per_channel_attribution")));
        Ok(())
    }

    #[test]
    fn capacity_report_quality_is_stable_for_same_seed() -> Result<()> {
        let dataset = tiny_dataset();
        let first = run_capacity_eval_for_dataset(
            CapacityEvalOptions {
                dataset_path: "inline-test".to_string(),
                seed: 11,
                scales: vec![1, 2],
                k: 5,
            },
            dataset.clone(),
        )?;
        let second = run_capacity_eval_for_dataset(
            CapacityEvalOptions {
                dataset_path: "inline-test".to_string(),
                seed: 11,
                scales: vec![1, 2],
                k: 5,
            },
            dataset,
        )?;

        assert_eq!(quality_signature(&first), quality_signature(&second));
        Ok(())
    }

    #[test]
    fn capacity_scales_require_one_x_baseline() {
        let err = normalize_scales(vec![2, 3])
            .expect_err("missing 1x scale should fail")
            .to_string();
        assert!(err.contains("requires scale 1"));
    }

    fn quality_signature(
        report: &CapacityEvalReport,
    ) -> Vec<(
        usize,
        String,
        usize,
        f64,
        f64,
        f64,
        Vec<(String, f64, f64, f64)>,
    )> {
        report
            .scales
            .iter()
            .map(|scale| {
                (
                    scale.scale,
                    scale.corpus_hash.clone(),
                    scale.fused.scored_queries,
                    scale.fused.recall_at_k,
                    scale.fused.ndcg_at_10,
                    scale.fused.evidence_recall_at_k,
                    scale
                        .channels
                        .iter()
                        .map(|(channel, metrics)| {
                            (
                                channel.clone(),
                                metrics.recall_at_k,
                                metrics.ndcg_at_10,
                                metrics.evidence_recall_at_k,
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }

    fn tiny_dataset() -> GoldenDataset {
        GoldenDataset {
            version: Some("capacity-test".to_string()),
            description: Some("capacity test fixture".to_string()),
            corpus: vec![
                GoldenMemory {
                    project: "synthetic/capacity".to_string(),
                    topic_key: Some("capacity-alpha-anchor".to_string()),
                    title: "Alpha routing anchor".to_string(),
                    content: "Alpha routing anchor lives in src/alpha.rs".to_string(),
                    memory_type: "decision".to_string(),
                    branch: Some("main".to_string()),
                    scope: "project".to_string(),
                    status: "active".to_string(),
                    files: Some("src/alpha.rs".to_string()),
                    created_at_epoch: Some(1_700_000_000),
                    access_count: Some(0),
                    last_accessed_epoch: None,
                },
                GoldenMemory {
                    project: "synthetic/capacity".to_string(),
                    topic_key: Some("capacity-beta-anchor".to_string()),
                    title: "Beta retry anchor".to_string(),
                    content: "Beta retry anchor belongs to src/beta.rs".to_string(),
                    memory_type: "bugfix".to_string(),
                    branch: Some("main".to_string()),
                    scope: "project".to_string(),
                    status: "active".to_string(),
                    files: Some("src/beta.rs".to_string()),
                    created_at_epoch: Some(1_700_000_010),
                    access_count: Some(0),
                    last_accessed_epoch: None,
                },
            ],
            queries: vec![
                GoldenQuery {
                    id: "alpha".to_string(),
                    query: "Alpha routing anchor".to_string(),
                    category: "retrieval".to_string(),
                    slice: Some("capacity_test".to_string()),
                    hop_path: None,
                    project: Some("synthetic/capacity".to_string()),
                    branch: Some("main".to_string()),
                    memory_type: None,
                    relevant_ids: vec![],
                    evidence_refs: vec![EvidenceRef {
                        topic_key: Some("capacity-alpha-anchor".to_string()),
                        ..EvidenceRef::default()
                    }],
                    expect_abstain: false,
                    false_premise: false,
                    notes: None,
                },
                GoldenQuery {
                    id: "beta".to_string(),
                    query: "Beta retry anchor".to_string(),
                    category: "retrieval".to_string(),
                    slice: Some("capacity_test".to_string()),
                    hop_path: None,
                    project: Some("synthetic/capacity".to_string()),
                    branch: Some("main".to_string()),
                    memory_type: None,
                    relevant_ids: vec![],
                    evidence_refs: vec![EvidenceRef {
                        topic_key: Some("capacity-beta-anchor".to_string()),
                        ..EvidenceRef::default()
                    }],
                    expect_abstain: false,
                    false_premise: false,
                    notes: None,
                },
            ],
        }
    }
}
