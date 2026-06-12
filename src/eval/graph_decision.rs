use std::fmt::{Display, Formatter, Result as FmtResult};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;

use super::golden::{self, CategoryEvaluation, GoldenDataset, MetricAverages};

pub const DEFAULT_DATASET_PATH: &str = "eval/golden.json";
pub const DEFAULT_REPORT_PATH: &str = "eval/graph-decision/report.json";
const BENEFIT_THRESHOLD: f64 = 0.05;
const LATENCY_BUDGET_P95_MS: f64 = 1000.0;
const EPSILON: f64 = 0.000_001;

#[derive(Debug, Clone)]
pub struct GraphDecisionEvalOptions {
    pub dataset_path: String,
    pub k: usize,
}

impl Default for GraphDecisionEvalOptions {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            k: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionReport {
    pub version: String,
    pub dataset_path: String,
    pub k: usize,
    pub benefit_threshold: f64,
    pub latency_budget_p95_ms: f64,
    pub evaluated_channel: EvaluatedGraphChannel,
    pub graph_edges_evaluated: bool,
    pub graph_edges_retrieval_decision: GraphEdgesRetrievalDecision,
    pub decision: GraphDecision,
    pub decision_reason: String,
    pub standard: GraphDecisionArmReport,
    pub entity_bfs: GraphDecisionArmReport,
    pub deltas: GraphDecisionDeltas,
    pub checks: GraphDecisionChecks,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDecision {
    WireEntityBfsExperiment,
    KeepGraphEdgesFrozenPendingLiteralEval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatedGraphChannel {
    EntityBfsProxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgesRetrievalDecision {
    RemainFrozenPendingLiteralEval,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionArmReport {
    pub mode: GraphDecisionMode,
    pub overall: CategoryEvaluation,
    pub multi_hop_slice: CategoryEvaluation,
    pub non_multi_hop_slices: CategoryEvaluation,
    pub multi_hop_queries_with_two_or_more_hops: usize,
    pub query_summaries: Vec<GraphDecisionQuerySummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDecisionMode {
    Standard,
    EntityBfs,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionQuerySummary {
    pub id: String,
    pub slice: String,
    pub status: String,
    pub result_count: usize,
    pub retrieved_ids: Vec<i64>,
    pub matched_refs: usize,
    pub expected_refs: usize,
    pub retrieval_latency_ms: f64,
    pub hops: Option<u8>,
    pub entities_discovered: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionDeltas {
    pub multi_hop_recall_at_k: f64,
    pub multi_hop_evidence_recall_at_k: f64,
    pub multi_hop_ndcg_at_10: f64,
    pub non_multi_hop_recall_at_k: f64,
    pub non_multi_hop_evidence_recall_at_k: f64,
    pub non_multi_hop_ndcg_at_10: f64,
    pub p95_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionChecks {
    pub multi_hop_slice_present: bool,
    pub entity_bfs_two_hop_observed: bool,
    pub benefit_threshold_met: bool,
    pub non_multi_hop_zero_regression: bool,
    pub p95_latency_within_budget: bool,
    pub safe_to_wire_entity_bfs: bool,
    pub all_checks_passed: bool,
}

pub fn run_graph_decision_eval(options: GraphDecisionEvalOptions) -> Result<GraphDecisionReport> {
    let dataset = golden::load_dataset(&options.dataset_path)?;
    run_graph_decision_dataset(dataset, options.dataset_path, options.k)
}

fn run_graph_decision_dataset(
    dataset: GoldenDataset,
    dataset_path: String,
    requested_k: usize,
) -> Result<GraphDecisionReport> {
    if !dataset.has_fixture_corpus() {
        bail!("graph decision eval requires a fixture-backed golden dataset");
    }

    let k = requested_k.max(1);
    let standard = evaluate_arm(&dataset, k, GraphDecisionMode::Standard)?;
    let entity_bfs = evaluate_arm(&dataset, k, GraphDecisionMode::EntityBfs)?;
    ensure_required_slices(&standard, &entity_bfs)?;
    let deltas = build_deltas(&standard, &entity_bfs);
    let checks = build_checks(&standard, &entity_bfs, &deltas);
    let decision = if checks.safe_to_wire_entity_bfs {
        GraphDecision::WireEntityBfsExperiment
    } else {
        GraphDecision::KeepGraphEdgesFrozenPendingLiteralEval
    };
    let decision_reason = match decision {
        GraphDecision::WireEntityBfsExperiment => format!(
            "Entity BFS is safe to wire as an experiment: it improved multi-hop evidence recall by at least {:.0}%, preserved non-multi-hop quality, stayed within the p95 latency budget, and exercised two-hop expansion.",
            BENEFIT_THRESHOLD * 100.0
        ),
        GraphDecision::KeepGraphEdgesFrozenPendingLiteralEval => format!(
            "This gate evaluated the entity-BFS proxy path, not literal graph_edges traversal. The proxy did not satisfy all wire requirements: >= {:.0}% multi-hop evidence-recall gain, non-multi-hop zero regression, p95 latency <= {:.0}ms, and observed two-hop expansion. Keep graph_edges retrieval frozen pending a dedicated graph_edges A/B eval.",
            BENEFIT_THRESHOLD * 100.0,
            LATENCY_BUDGET_P95_MS
        ),
    };

    Ok(GraphDecisionReport {
        version: "2026-06-12".to_string(),
        dataset_path,
        k,
        benefit_threshold: BENEFIT_THRESHOLD,
        latency_budget_p95_ms: LATENCY_BUDGET_P95_MS,
        evaluated_channel: EvaluatedGraphChannel::EntityBfsProxy,
        graph_edges_evaluated: false,
        graph_edges_retrieval_decision:
            GraphEdgesRetrievalDecision::RemainFrozenPendingLiteralEval,
        decision,
        decision_reason,
        standard,
        entity_bfs,
        deltas,
        checks,
        notes: vec![
            "Entity BFS is the existing explicit multi-hop expansion through memory_entities plus FTS mention fallback; this report does not evaluate first-class graph_edges traversal.".to_string(),
            "The graph_edges retrieval decision is therefore a conservative freeze: graph_edges remains available for provenance/candidates but cannot become a retrieval channel until a future pre-registered literal graph_edges eval shows a material gain.".to_string(),
        ],
    })
}

pub fn ensure_graph_decision_gate(report: &GraphDecisionReport) -> Result<()> {
    if report.checks.all_checks_passed {
        return Ok(());
    }
    bail!(
        "graph decision eval failed: multi_hop_slice_present={} non_multi_hop_zero_regression={} p95_latency_within_budget={}",
        report.checks.multi_hop_slice_present,
        report.checks.non_multi_hop_zero_regression,
        report.checks.p95_latency_within_budget
    )
}

fn evaluate_arm(
    dataset: &GoldenDataset,
    k: usize,
    mode: GraphDecisionMode,
) -> Result<GraphDecisionArmReport> {
    let conn = Connection::open_in_memory().context("open in-memory graph decision eval DB")?;
    crate::migrate::run_migrations(&conn).context("migrate graph decision eval DB")?;
    golden::run::seed_fixture_corpus(&conn, &dataset.corpus)?;

    let mut overall = golden::run::CategoryAccumulator::default();
    let mut multi_hop_slice = golden::run::CategoryAccumulator::default();
    let mut non_multi_hop_slices = golden::run::CategoryAccumulator::default();
    let mut query_summaries = Vec::with_capacity(dataset.queries.len());

    for query in &dataset.queries {
        let started = Instant::now();
        let (results, hops, entities_discovered) = match mode {
            GraphDecisionMode::Standard => (
                crate::retrieval::search::search_with_branch(
                    &conn,
                    Some(&query.query),
                    query.project.as_deref(),
                    query.memory_type.as_deref(),
                    k.max(10) as i64,
                    0,
                    false,
                    query.branch.as_deref(),
                )?,
                None,
                Vec::new(),
            ),
            GraphDecisionMode::EntityBfs => {
                let multi_hop = crate::retrieval::search_multihop::search_multi_hop(
                    &conn,
                    &query.query,
                    query.project.as_deref(),
                    k.max(10) as i64,
                    0,
                    query.memory_type.as_deref(),
                    query.branch.as_deref(),
                    false,
                )?;
                (
                    multi_hop.memories,
                    Some(multi_hop.hops),
                    multi_hop.entities_discovered,
                )
            }
        };
        let retrieval_latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let query_tokens = golden::run::estimate_query_tokens(&query.query);
        let evaluation =
            golden::run::evaluate_query(query, &results, k, query_tokens, retrieval_latency_ms);

        golden::run::record_bucket(&mut overall, query, &evaluation);
        if query.slice_label() == "multi_hop" {
            golden::run::record_bucket(&mut multi_hop_slice, query, &evaluation);
        } else {
            golden::run::record_bucket(&mut non_multi_hop_slices, query, &evaluation);
        }
        query_summaries.push(GraphDecisionQuerySummary {
            id: evaluation.id.clone(),
            slice: evaluation.slice.clone(),
            status: evaluation.status.label().to_string(),
            result_count: evaluation.result_count,
            retrieved_ids: evaluation.retrieved_ids.clone(),
            matched_refs: evaluation.matched_refs,
            expected_refs: evaluation.expected_refs,
            retrieval_latency_ms,
            hops,
            entities_discovered,
        });
    }

    let multi_hop_queries_with_two_or_more_hops = query_summaries
        .iter()
        .filter(|summary| {
            summary.slice == "multi_hop" && summary.hops.is_some_and(|hops| hops >= 2)
        })
        .count();

    Ok(GraphDecisionArmReport {
        mode,
        overall: golden::run::bucket_evaluation(overall),
        multi_hop_slice: golden::run::bucket_evaluation(multi_hop_slice),
        non_multi_hop_slices: golden::run::bucket_evaluation(non_multi_hop_slices),
        multi_hop_queries_with_two_or_more_hops,
        query_summaries,
    })
}

fn ensure_required_slices(
    standard: &GraphDecisionArmReport,
    entity_bfs: &GraphDecisionArmReport,
) -> Result<()> {
    if standard.multi_hop_slice.scored_queries == 0
        || entity_bfs.multi_hop_slice.scored_queries == 0
    {
        bail!("graph decision eval requires at least one scored multi_hop query in both arms");
    }
    Ok(())
}

fn build_deltas(
    standard: &GraphDecisionArmReport,
    entity_bfs: &GraphDecisionArmReport,
) -> GraphDecisionDeltas {
    GraphDecisionDeltas {
        multi_hop_recall_at_k: metric_delta(
            standard.multi_hop_slice.metrics.as_ref(),
            entity_bfs.multi_hop_slice.metrics.as_ref(),
            |m| m.recall_at_k,
        ),
        multi_hop_evidence_recall_at_k: metric_delta(
            standard.multi_hop_slice.metrics.as_ref(),
            entity_bfs.multi_hop_slice.metrics.as_ref(),
            |m| m.evidence_recall_at_k,
        ),
        multi_hop_ndcg_at_10: metric_delta(
            standard.multi_hop_slice.metrics.as_ref(),
            entity_bfs.multi_hop_slice.metrics.as_ref(),
            |m| m.ndcg_at_10,
        ),
        non_multi_hop_recall_at_k: metric_delta(
            standard.non_multi_hop_slices.metrics.as_ref(),
            entity_bfs.non_multi_hop_slices.metrics.as_ref(),
            |m| m.recall_at_k,
        ),
        non_multi_hop_evidence_recall_at_k: metric_delta(
            standard.non_multi_hop_slices.metrics.as_ref(),
            entity_bfs.non_multi_hop_slices.metrics.as_ref(),
            |m| m.evidence_recall_at_k,
        ),
        non_multi_hop_ndcg_at_10: metric_delta(
            standard.non_multi_hop_slices.metrics.as_ref(),
            entity_bfs.non_multi_hop_slices.metrics.as_ref(),
            |m| m.ndcg_at_10,
        ),
        p95_latency_ms: entity_bfs.overall.retrieval_latency_p95_ms
            - standard.overall.retrieval_latency_p95_ms,
    }
}

fn metric_delta(
    standard: Option<&MetricAverages>,
    entity_bfs: Option<&MetricAverages>,
    value: impl Fn(&MetricAverages) -> f64,
) -> f64 {
    match (standard, entity_bfs) {
        (Some(standard), Some(entity_bfs)) => value(entity_bfs) - value(standard),
        _ => 0.0,
    }
}

fn build_checks(
    standard: &GraphDecisionArmReport,
    entity_bfs: &GraphDecisionArmReport,
    deltas: &GraphDecisionDeltas,
) -> GraphDecisionChecks {
    let multi_hop_slice_present = standard.multi_hop_slice.scored_queries > 0
        && entity_bfs.multi_hop_slice.scored_queries > 0;
    let entity_bfs_two_hop_observed = entity_bfs.multi_hop_queries_with_two_or_more_hops > 0;
    let benefit_threshold_met = deltas.multi_hop_evidence_recall_at_k >= BENEFIT_THRESHOLD;
    let non_multi_hop_zero_regression = metrics_not_lower(
        standard.non_multi_hop_slices.metrics.as_ref(),
        entity_bfs.non_multi_hop_slices.metrics.as_ref(),
    ) && entity_bfs.non_multi_hop_slices.abstention_passed
        >= standard.non_multi_hop_slices.abstention_passed;
    let p95_latency_within_budget =
        entity_bfs.overall.retrieval_latency_p95_ms <= LATENCY_BUDGET_P95_MS;
    let safe_to_wire_entity_bfs = benefit_threshold_met
        && non_multi_hop_zero_regression
        && p95_latency_within_budget
        && entity_bfs_two_hop_observed;

    GraphDecisionChecks {
        multi_hop_slice_present,
        entity_bfs_two_hop_observed,
        benefit_threshold_met,
        non_multi_hop_zero_regression,
        p95_latency_within_budget,
        safe_to_wire_entity_bfs,
        all_checks_passed: multi_hop_slice_present
            && non_multi_hop_zero_regression
            && p95_latency_within_budget,
    }
}

fn metrics_not_lower(
    standard: Option<&MetricAverages>,
    entity_bfs: Option<&MetricAverages>,
) -> bool {
    match (standard, entity_bfs) {
        (Some(standard), Some(entity_bfs)) => {
            entity_bfs.hit_at_k + EPSILON >= standard.hit_at_k
                && entity_bfs.mrr_at_10 + EPSILON >= standard.mrr_at_10
                && entity_bfs.precision_at_k + EPSILON >= standard.precision_at_k
                && entity_bfs.recall_at_k + EPSILON >= standard.recall_at_k
                && entity_bfs.ndcg_at_10 + EPSILON >= standard.ndcg_at_10
                && entity_bfs.evidence_recall_at_k + EPSILON >= standard.evidence_recall_at_k
        }
        (None, None) => true,
        _ => false,
    }
}

impl Display for GraphDecisionReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(
            f,
            "remem graph decision eval — {:?}, k={}, threshold={:.2}",
            self.decision, self.k, self.benefit_threshold
        )?;
        writeln!(f, "reason: {}", self.decision_reason)?;
        writeln!(
            f,
            "multi-hop evidence delta={:.3}, non-multi-hop evidence delta={:.3}, entity-bfs p95={:.2}ms",
            self.deltas.multi_hop_evidence_recall_at_k,
            self.deltas.non_multi_hop_evidence_recall_at_k,
            self.entity_bfs.overall.retrieval_latency_p95_ms
        )?;
        writeln!(
            f,
            "checks: multi_hop_slice_present={} entity_bfs_two_hop_observed={} benefit_threshold_met={} non_multi_hop_zero_regression={} p95_latency_within_budget={} safe_to_wire_entity_bfs={} all_checks_passed={}",
            self.checks.multi_hop_slice_present,
            self.checks.entity_bfs_two_hop_observed,
            self.checks.benefit_threshold_met,
            self.checks.non_multi_hop_zero_regression,
            self.checks.p95_latency_within_budget,
            self.checks.safe_to_wire_entity_bfs,
            self.checks.all_checks_passed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_decision_eval_freezes_without_material_gain() -> Result<()> {
        let report = run_graph_decision_eval(GraphDecisionEvalOptions::default())?;
        assert_eq!(
            report.decision,
            GraphDecision::KeepGraphEdgesFrozenPendingLiteralEval
        );
        assert_eq!(
            report.evaluated_channel,
            EvaluatedGraphChannel::EntityBfsProxy
        );
        assert!(!report.graph_edges_evaluated);
        assert_eq!(
            report.graph_edges_retrieval_decision,
            GraphEdgesRetrievalDecision::RemainFrozenPendingLiteralEval
        );
        assert!(report.checks.all_checks_passed);
        assert!(!report.checks.safe_to_wire_entity_bfs);
        assert!(report.deltas.multi_hop_evidence_recall_at_k < BENEFIT_THRESHOLD);
        Ok(())
    }

    #[test]
    fn graph_decision_eval_rejects_dataset_without_multi_hop_slice() -> Result<()> {
        let mut dataset = golden::load_dataset(DEFAULT_DATASET_PATH)?;
        for query in &mut dataset.queries {
            if query.slice_label() == "multi_hop" {
                query.slice = Some("paraphrase".to_string());
            }
        }

        let error = run_graph_decision_dataset(
            dataset,
            DEFAULT_DATASET_PATH.to_string(),
            GraphDecisionEvalOptions::default().k,
        )
        .expect_err("dataset without multi_hop slice must fail the graph decision gate");

        assert!(error
            .to_string()
            .contains("requires at least one scored multi_hop query"));
        Ok(())
    }
}
