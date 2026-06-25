use std::collections::BTreeMap;
use std::fmt::{Display, Formatter, Result as FmtResult};

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;

use super::golden::{self, CategoryEvaluation, GoldenDataset, MetricAverages};
use crate::retrieval::search::SearchWeights;

mod usage_shadow;

pub const DEFAULT_DATASET_PATH: &str = "eval/golden.json";
pub const DEFAULT_REPORT_PATH: &str = "eval/weight-grid/report.json";
const EPSILON: f64 = 0.000_001;
const MIN_RECALL_AT_K_DEFAULT_FLIP_DELTA: f64 = 0.05;

#[derive(Debug, Clone)]
pub struct WeightGridOptions {
    pub dataset_path: String,
    pub k: usize,
}

impl Default for WeightGridOptions {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            k: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WeightGridReport {
    pub version: String,
    pub dataset_path: String,
    pub k: usize,
    pub scoring: WeightGridScoring,
    pub default_weights: SearchWeights,
    pub default_rank: usize,
    pub default_score: f64,
    pub best: WeightGridCandidate,
    pub recommendation: WeightGridRecommendation,
    pub checks: WeightGridChecks,
    pub usage_shadow: usage_shadow::UsageShadowReport,
    pub candidates: Vec<WeightGridCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeightGridScoring {
    pub evidence_recall_weight: f64,
    pub hit_weight: f64,
    pub ndcg_weight: f64,
    pub mrr_weight: f64,
    pub abstention_weight: f64,
}

impl Default for WeightGridScoring {
    fn default() -> Self {
        Self {
            evidence_recall_weight: 4.0,
            hit_weight: 3.0,
            ndcg_weight: 2.0,
            mrr_weight: 1.0,
            abstention_weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeightGridRecommendation {
    KeepShippedDefaults,
    CandidateImprovesSecondaryMetricOnlyKeepDefaults,
    CandidateOutperformsDefaultsNeedsDecision,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeightGridChecks {
    pub fixture_corpus_used: bool,
    pub default_weights_in_grid: bool,
    pub best_preserves_abstention: bool,
    pub best_preserves_scored_query_count: bool,
    pub best_meets_recall_at_k_default_flip_gate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeightGridCandidate {
    pub rank: usize,
    pub weights: SearchWeights,
    pub score: f64,
    pub distance_from_defaults: f64,
    pub deltas_vs_default: WeightGridDeltas,
    pub overall: CategoryEvaluation,
    pub by_slice: BTreeMap<String, CategoryEvaluation>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WeightGridDeltas {
    pub hit_at_k: f64,
    pub mrr_at_10: f64,
    pub precision_at_k: f64,
    pub recall_at_k: f64,
    pub ndcg_at_10: f64,
    pub evidence_recall_at_k: f64,
    pub abstention_pass_rate: f64,
}

pub fn run_weight_grid(options: WeightGridOptions) -> Result<WeightGridReport> {
    let dataset = golden::load_dataset(&options.dataset_path)?;
    run_weight_grid_dataset(
        dataset,
        options.dataset_path,
        options.k,
        default_candidate_grid(),
    )
}

fn run_weight_grid_dataset(
    dataset: GoldenDataset,
    dataset_path: String,
    requested_k: usize,
    candidates: Vec<SearchWeights>,
) -> Result<WeightGridReport> {
    if !dataset.has_fixture_corpus() {
        bail!("weight grid eval requires a fixture-backed golden dataset");
    }
    if candidates.is_empty() {
        bail!("weight grid requires at least one candidate");
    }

    let k = requested_k.max(1);
    let conn = Connection::open_in_memory().context("open in-memory weight grid eval DB")?;
    crate::migrate::run_migrations(&conn).context("migrate weight grid eval DB")?;
    golden::run::seed_fixture_corpus(&conn, &dataset.corpus)?;

    let default_weights = SearchWeights::default();
    let scoring = WeightGridScoring::default();
    let mut evaluated = Vec::with_capacity(candidates.len());
    for weights in candidates {
        evaluated.push(evaluate_candidate(&conn, &dataset, k, weights, &scoring)?);
    }
    let default_index = evaluated
        .iter()
        .position(|candidate| candidate.weights == default_weights)
        .context("default search weights were not included in the grid")?;
    let default_snapshot = evaluated[default_index].clone();
    let default_score = default_snapshot.score;
    let default_overall = default_snapshot.overall.clone();

    for candidate in &mut evaluated {
        candidate.distance_from_defaults = weight_distance(candidate.weights, default_weights);
        candidate.deltas_vs_default = build_candidate_deltas(&default_overall, &candidate.overall);
    }
    evaluated.sort_by(compare_candidates);
    for (index, candidate) in evaluated.iter_mut().enumerate() {
        candidate.rank = index + 1;
    }

    let default_rank = evaluated
        .iter()
        .find(|candidate| candidate.weights == default_weights)
        .map(|candidate| candidate.rank)
        .context("default search weights disappeared after sorting")?;
    let best = evaluated
        .first()
        .cloned()
        .context("weight grid produced no evaluated candidates")?;
    let best_meets_recall_at_k_default_flip_gate =
        candidate_meets_recall_at_k_default_flip_gate(&best);
    let recommendation = if best.weights == default_weights || best.score <= default_score + EPSILON
    {
        WeightGridRecommendation::KeepShippedDefaults
    } else if best_meets_recall_at_k_default_flip_gate {
        WeightGridRecommendation::CandidateOutperformsDefaultsNeedsDecision
    } else {
        WeightGridRecommendation::CandidateImprovesSecondaryMetricOnlyKeepDefaults
    };
    let checks = WeightGridChecks {
        fixture_corpus_used: true,
        default_weights_in_grid: true,
        best_preserves_abstention: best.overall.abstention_passed
            >= default_overall.abstention_passed,
        best_preserves_scored_query_count: best.overall.scored_queries
            >= default_overall.scored_queries,
        best_meets_recall_at_k_default_flip_gate,
    };
    let usage_shadow = usage_shadow::build_usage_shadow_report(&conn, &dataset, k)?;

    Ok(WeightGridReport {
        version: "2026-06-23".to_string(),
        dataset_path,
        k,
        scoring,
        default_weights,
        default_rank,
        default_score,
        best,
        recommendation,
        checks,
        usage_shadow,
        candidates: evaluated,
    })
}

fn evaluate_candidate(
    conn: &Connection,
    dataset: &GoldenDataset,
    k: usize,
    weights: SearchWeights,
    scoring: &WeightGridScoring,
) -> Result<WeightGridCandidate> {
    let mut overall = golden::run::CategoryAccumulator::default();
    let mut by_slice = BTreeMap::<String, golden::run::CategoryAccumulator>::new();
    let fetch_limit = k.max(10) as i64;

    for query in &dataset.queries {
        let results = crate::retrieval::search::search_with_branch_weights(
            conn,
            Some(&query.query),
            query.project.as_deref(),
            query.memory_type.as_deref(),
            fetch_limit,
            0,
            false,
            query.branch.as_deref(),
            weights,
        )?;
        let query_tokens = golden::run::estimate_query_tokens(&query.query);
        let evaluation = golden::run::evaluate_query(query, &results, k, query_tokens, 0.0);
        golden::run::record_bucket(&mut overall, query, &evaluation);
        golden::run::record_bucket(
            by_slice.entry(query.slice_label().to_string()).or_default(),
            query,
            &evaluation,
        );
    }

    let overall = golden::run::bucket_evaluation(overall);
    let score = candidate_score(&overall, scoring);
    Ok(WeightGridCandidate {
        rank: 0,
        weights,
        score,
        distance_from_defaults: 0.0,
        deltas_vs_default: WeightGridDeltas::default(),
        overall,
        by_slice: by_slice
            .into_iter()
            .map(|(slice, bucket)| (slice, golden::run::bucket_evaluation(bucket)))
            .collect(),
    })
}

fn default_candidate_grid() -> Vec<SearchWeights> {
    let default = SearchWeights::default();
    let mut candidates = Vec::new();
    for fts in [2.0, default.fts, 3.0] {
        for vector in [2.5, default.vector, 3.5] {
            for entity in [1.0, default.entity, 1.5] {
                for temporal in [0.75, default.temporal, 1.25] {
                    for like_fallback in [0.1, default.like_fallback] {
                        push_unique(
                            &mut candidates,
                            SearchWeights {
                                fts,
                                vector,
                                entity,
                                temporal,
                                like_fallback,
                                ..default
                            },
                        );
                    }
                }
            }
        }
    }
    for min_evidence_confidence in [0.0, 0.5, default.min_evidence_confidence, 0.75, 1.0] {
        push_unique(
            &mut candidates,
            SearchWeights {
                min_evidence_confidence,
                ..default
            },
        );
    }
    for fact in [0.0, default.fact, 1.8] {
        push_unique(&mut candidates, SearchWeights { fact, ..default });
    }
    for usage in [0.25, 0.75, 1.5] {
        push_unique(&mut candidates, SearchWeights { usage, ..default });
    }
    candidates
}

fn push_unique(candidates: &mut Vec<SearchWeights>, weights: SearchWeights) {
    if !candidates.contains(&weights) {
        candidates.push(weights);
    }
}

fn compare_candidates(
    left: &WeightGridCandidate,
    right: &WeightGridCandidate,
) -> std::cmp::Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| {
            left.distance_from_defaults
                .total_cmp(&right.distance_from_defaults)
        })
        .then_with(|| left.weights.fts.total_cmp(&right.weights.fts))
        .then_with(|| left.weights.vector.total_cmp(&right.weights.vector))
        .then_with(|| left.weights.entity.total_cmp(&right.weights.entity))
        .then_with(|| left.weights.temporal.total_cmp(&right.weights.temporal))
        .then_with(|| left.weights.fact.total_cmp(&right.weights.fact))
        .then_with(|| {
            left.weights
                .like_fallback
                .total_cmp(&right.weights.like_fallback)
        })
        .then_with(|| left.weights.usage.total_cmp(&right.weights.usage))
        .then_with(|| {
            left.weights
                .usage_recency_half_life_days
                .total_cmp(&right.weights.usage_recency_half_life_days)
        })
        .then_with(|| {
            left.weights
                .min_evidence_confidence
                .total_cmp(&right.weights.min_evidence_confidence)
        })
}

fn candidate_score(overall: &CategoryEvaluation, scoring: &WeightGridScoring) -> f64 {
    let metric_score = overall.metrics.as_ref().map_or(0.0, |metrics| {
        scoring.evidence_recall_weight * metrics.evidence_recall_at_k
            + scoring.hit_weight * metrics.hit_at_k
            + scoring.ndcg_weight * metrics.ndcg_at_10
            + scoring.mrr_weight * metrics.mrr_at_10
    });
    metric_score + scoring.abstention_weight * abstention_pass_rate(overall)
}

fn abstention_pass_rate(evaluation: &CategoryEvaluation) -> f64 {
    if evaluation.abstention_queries == 0 {
        1.0
    } else {
        evaluation.abstention_passed as f64 / evaluation.abstention_queries as f64
    }
}

fn candidate_meets_recall_at_k_default_flip_gate(candidate: &WeightGridCandidate) -> bool {
    candidate.deltas_vs_default.recall_at_k >= MIN_RECALL_AT_K_DEFAULT_FLIP_DELTA
        || candidate.deltas_vs_default.evidence_recall_at_k >= MIN_RECALL_AT_K_DEFAULT_FLIP_DELTA
}

fn build_candidate_deltas(
    default_overall: &CategoryEvaluation,
    candidate_overall: &CategoryEvaluation,
) -> WeightGridDeltas {
    WeightGridDeltas {
        hit_at_k: metric_average_delta(
            default_overall.metrics.as_ref(),
            candidate_overall.metrics.as_ref(),
            |m| m.hit_at_k,
        ),
        mrr_at_10: metric_average_delta(
            default_overall.metrics.as_ref(),
            candidate_overall.metrics.as_ref(),
            |m| m.mrr_at_10,
        ),
        precision_at_k: metric_average_delta(
            default_overall.metrics.as_ref(),
            candidate_overall.metrics.as_ref(),
            |m| m.precision_at_k,
        ),
        recall_at_k: metric_average_delta(
            default_overall.metrics.as_ref(),
            candidate_overall.metrics.as_ref(),
            |m| m.recall_at_k,
        ),
        ndcg_at_10: metric_average_delta(
            default_overall.metrics.as_ref(),
            candidate_overall.metrics.as_ref(),
            |m| m.ndcg_at_10,
        ),
        evidence_recall_at_k: metric_average_delta(
            default_overall.metrics.as_ref(),
            candidate_overall.metrics.as_ref(),
            |m| m.evidence_recall_at_k,
        ),
        abstention_pass_rate: abstention_pass_rate(candidate_overall)
            - abstention_pass_rate(default_overall),
    }
}

fn metric_average_delta(
    default: Option<&MetricAverages>,
    candidate: Option<&MetricAverages>,
    value: impl Fn(&MetricAverages) -> f64,
) -> f64 {
    match (default, candidate) {
        (Some(default), Some(candidate)) => value(candidate) - value(default),
        (None, Some(candidate)) => value(candidate),
        (Some(default), None) => -value(default),
        (None, None) => 0.0,
    }
}

fn weight_distance(candidate: SearchWeights, default: SearchWeights) -> f64 {
    (candidate.fts - default.fts).abs()
        + (candidate.vector - default.vector).abs()
        + (candidate.entity - default.entity).abs()
        + (candidate.temporal - default.temporal).abs()
        + (candidate.fact - default.fact).abs()
        + (candidate.like_fallback - default.like_fallback).abs()
        + (candidate.usage - default.usage).abs()
        + (candidate.usage_recency_half_life_days - default.usage_recency_half_life_days).abs()
        + f64::from((candidate.max_vector_distance - default.max_vector_distance).abs())
        + (candidate.rrf_k - default.rrf_k).abs()
        + (candidate.min_evidence_confidence - default.min_evidence_confidence).abs()
}

impl Display for WeightGridReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(
            f,
            "remem weight grid — {} candidates, k={}, default_rank={}, recommendation={:?}",
            self.candidates.len(),
            self.k,
            self.default_rank,
            self.recommendation
        )?;
        writeln!(f, "dataset: {}", self.dataset_path)?;
        writeln!(
            f,
            "default score={:.4}, best score={:.4}",
            self.default_score, self.best.score
        )?;
        writeln!(f)?;
        writeln!(f, "--- Top Candidates ---")?;
        for candidate in self.candidates.iter().take(10) {
            writeln!(
                f,
                "  #{:02} score={:.4} dist={:.2} fts={:.2} vector={:.2} entity={:.2} temporal={:.2} fact={:.2} like={:.2} usage={:.2} confidence={:.2}",
                candidate.rank,
                candidate.score,
                candidate.distance_from_defaults,
                candidate.weights.fts,
                candidate.weights.vector,
                candidate.weights.entity,
                candidate.weights.temporal,
                candidate.weights.fact,
                candidate.weights.like_fallback,
                candidate.weights.usage,
                candidate.weights.min_evidence_confidence
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::golden::{EvidenceRef, GoldenMemory, GoldenQuery};

    #[test]
    fn grid_report_includes_defaults_and_ranks_candidates() -> Result<()> {
        let default = SearchWeights::default();
        let dataset = GoldenDataset {
            version: Some("test".to_string()),
            description: None,
            corpus: vec![GoldenMemory {
                project: "/repo-a".to_string(),
                topic_key: Some("sqlcipher-store".to_string()),
                title: "SQLCipher store".to_string(),
                content: "SQLCipher encrypts the local memory database at rest.".to_string(),
                memory_type: "architecture".to_string(),
                branch: Some("main".to_string()),
                scope: "project".to_string(),
                status: "active".to_string(),
                files: None,
                created_at_epoch: Some(1),
                access_count: None,
                last_accessed_epoch: None,
            }],
            queries: vec![GoldenQuery {
                id: "q1".to_string(),
                query: "local database encryption".to_string(),
                category: "retrieval".to_string(),
                slice: Some("paraphrase".to_string()),
                project: Some("/repo-a".to_string()),
                branch: Some("main".to_string()),
                memory_type: None,
                relevant_ids: vec![],
                evidence_refs: vec![EvidenceRef {
                    topic_key: Some("sqlcipher-store".to_string()),
                    text_contains: Some("encrypts the local memory database".to_string()),
                    ..EvidenceRef::default()
                }],
                expect_abstain: false,
                false_premise: false,
                notes: None,
            }],
        };
        let report = run_weight_grid_dataset(
            dataset,
            "test-golden.json".to_string(),
            5,
            vec![
                default,
                SearchWeights {
                    vector: default.vector + 0.5,
                    ..default
                },
            ],
        )?;

        assert_eq!(report.candidates.len(), 2);
        assert!(report.checks.default_weights_in_grid);
        assert!(report.default_rank >= 1);
        assert!(report
            .candidates
            .iter()
            .any(|candidate| candidate.weights == default));
        assert!(report.usage_shadow.default_usage_weight_zero);
        assert!(report
            .usage_shadow
            .comparisons
            .iter()
            .all(|comparison| comparison.usage_weight > 0.0));
        assert_eq!(report.candidates[0].rank, 1);
        Ok(())
    }
}
