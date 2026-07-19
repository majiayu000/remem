use std::collections::{BTreeMap, BTreeSet};
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
    pub literal_graph: GraphDecisionArmReport,
    pub deltas: GraphDecisionDeltas,
    pub checks: GraphDecisionChecks,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDecision {
    WireLiteralGraphTraversal,
    KeepGraphEdgesFrozenPendingLiteralEval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatedGraphChannel {
    LiteralGraphEdges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgesRetrievalDecision {
    WireProductionChannel,
    RemainFrozenPendingLiteralEval,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionArmReport {
    pub mode: GraphDecisionMode,
    pub overall: CategoryEvaluation,
    pub associative_slice: CategoryEvaluation,
    pub non_associative_slices: CategoryEvaluation,
    pub non_associative_by_slice: BTreeMap<String, CategoryEvaluation>,
    pub associative_queries_with_two_or_more_hops: usize,
    pub scope_leak_count: usize,
    pub query_summaries: Vec<GraphDecisionQuerySummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDecisionMode {
    Standard,
    EntityBfs,
    LiteralGraph,
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
    pub graph_result_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionDeltas {
    pub associative_recall_at_k: f64,
    pub associative_evidence_recall_at_k: f64,
    pub associative_ndcg_at_10: f64,
    pub non_associative_recall_at_k: f64,
    pub non_associative_evidence_recall_at_k: f64,
    pub non_associative_ndcg_at_10: f64,
    pub p95_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDecisionChecks {
    pub associative_slice_present: bool,
    pub literal_two_hop_observed: bool,
    pub benefit_threshold_met: bool,
    pub non_associative_zero_regression: bool,
    pub zero_scope_leak: bool,
    pub p95_latency_within_budget: bool,
    pub safe_to_wire_literal_graph: bool,
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
    let literal_graph = evaluate_arm(&dataset, k, GraphDecisionMode::LiteralGraph)?;
    ensure_required_slices(&standard, &literal_graph)?;
    let deltas = build_deltas(&standard, &literal_graph);
    let checks = build_checks(&standard, &literal_graph, &deltas);
    let decision = if checks.safe_to_wire_literal_graph {
        GraphDecision::WireLiteralGraphTraversal
    } else {
        GraphDecision::KeepGraphEdgesFrozenPendingLiteralEval
    };
    let decision_reason = match decision {
        GraphDecision::WireLiteralGraphTraversal => format!(
            "Literal graph_edges traversal is safe to wire: it improved associative evidence recall by at least {:.0}%, preserved non-associative quality, produced no scope leak, stayed within the p95 latency budget, and exercised a real two-edge path.",
            BENEFIT_THRESHOLD * 100.0
        ),
        GraphDecision::KeepGraphEdgesFrozenPendingLiteralEval => format!(
            "Literal graph_edges traversal did not satisfy all wire requirements: >= {:.0}% associative evidence-recall gain, non-associative zero regression, zero scope leak, p95 latency <= {:.0}ms, and an observed two-edge expansion. Keep graph_edges retrieval frozen.",
            BENEFIT_THRESHOLD * 100.0,
            LATENCY_BUDGET_P95_MS
        ),
    };

    Ok(GraphDecisionReport {
        version: "2026-07-19".to_string(),
        dataset_path,
        k,
        benefit_threshold: BENEFIT_THRESHOLD,
        latency_budget_p95_ms: LATENCY_BUDGET_P95_MS,
        evaluated_channel: EvaluatedGraphChannel::LiteralGraphEdges,
        graph_edges_evaluated: true,
        graph_edges_retrieval_decision: if checks.safe_to_wire_literal_graph {
            GraphEdgesRetrievalDecision::WireProductionChannel
        } else {
            GraphEdgesRetrievalDecision::RemainFrozenPendingLiteralEval
        },
        decision,
        decision_reason,
        standard,
        entity_bfs,
        literal_graph,
        deltas,
        checks,
        notes: vec![
            "The standard and literal arms use the same golden dataset and search implementation; the standard arm sets graph weight to zero.".to_string(),
            "Associative hop_path metadata seeds trusted mentions/touches_file edges through the typed provenance contract before literal-arm queries run.".to_string(),
            "Entity BFS remains informational and does not decide whether literal graph_edges traversal is wired.".to_string(),
        ],
    })
}

pub fn ensure_graph_decision_gate(report: &GraphDecisionReport) -> Result<()> {
    if report.checks.all_checks_passed {
        return Ok(());
    }
    bail!(
        "graph decision eval failed: associative_slice_present={} non_associative_zero_regression={} zero_scope_leak={} p95_latency_within_budget={}",
        report.checks.associative_slice_present,
        report.checks.non_associative_zero_regression,
        report.checks.zero_scope_leak,
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
    if mode == GraphDecisionMode::LiteralGraph {
        seed_fixture_graph_edges(&conn, dataset)?;
    }

    let mut overall = golden::run::CategoryAccumulator::default();
    let mut associative_slice = golden::run::CategoryAccumulator::default();
    let mut non_associative_slices = golden::run::CategoryAccumulator::default();
    let mut non_associative_by_slice = BTreeMap::<String, golden::run::CategoryAccumulator>::new();
    let mut query_summaries = Vec::with_capacity(dataset.queries.len());
    let mut scope_leak_count = 0;

    for query in &dataset.queries {
        let started = Instant::now();
        let (results, hops, entities_discovered, graph_result_count) = match mode {
            GraphDecisionMode::Standard => (
                crate::retrieval::search::search_with_branch_weights(
                    &conn,
                    Some(&query.query),
                    query.project.as_deref(),
                    query.memory_type.as_deref(),
                    k.max(10) as i64,
                    0,
                    false,
                    query.branch.as_deref(),
                    crate::retrieval::search::SearchWeights {
                        graph: 0.0,
                        ..crate::retrieval::search::SearchWeights::default()
                    },
                )?,
                None,
                Vec::new(),
                0,
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
                    false,
                )?;
                (
                    multi_hop.memories,
                    Some(multi_hop.hops),
                    multi_hop.entities_discovered,
                    0,
                )
            }
            GraphDecisionMode::LiteralGraph => {
                let (results, explain) = crate::retrieval::search::search_with_branch_explain(
                    &conn,
                    Some(&query.query),
                    query.project.as_deref(),
                    query.memory_type.as_deref(),
                    k.max(10) as i64,
                    0,
                    false,
                    query.branch.as_deref(),
                )?;
                let (hops, graph_result_count) = literal_path_summary(
                    &conn,
                    query,
                    &results,
                    explain
                        .as_ref()
                        .context("literal graph search missing explain")?,
                )?;
                (results, hops, Vec::new(), graph_result_count)
            }
        };
        let retrieval_latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let query_tokens = golden::run::estimate_query_tokens(&query.query);
        let evaluation =
            golden::run::evaluate_query(query, &results, k, query_tokens, retrieval_latency_ms);

        golden::run::record_bucket(&mut overall, query, &evaluation);
        if query.slice_label() == "associative" {
            golden::run::record_bucket(&mut associative_slice, query, &evaluation);
        } else {
            golden::run::record_bucket(&mut non_associative_slices, query, &evaluation);
            golden::run::record_bucket(
                non_associative_by_slice
                    .entry(query.slice_label().to_string())
                    .or_default(),
                query,
                &evaluation,
            );
        }
        scope_leak_count += results
            .iter()
            .filter(|memory| memory.scope != "global")
            .filter(|memory| {
                query.project.as_deref().is_some_and(|project| {
                    !crate::project_id::project_matches(Some(&memory.project), project)
                })
            })
            .count();
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
            graph_result_count,
        });
    }

    let associative_queries_with_two_or_more_hops = query_summaries
        .iter()
        .filter(|summary| {
            summary.slice == "associative" && summary.hops.is_some_and(|hops| hops >= 2)
        })
        .count();

    Ok(GraphDecisionArmReport {
        mode,
        overall: golden::run::bucket_evaluation(overall),
        associative_slice: golden::run::bucket_evaluation(associative_slice),
        non_associative_slices: golden::run::bucket_evaluation(non_associative_slices),
        non_associative_by_slice: non_associative_by_slice
            .into_iter()
            .map(|(name, bucket)| (name, golden::run::bucket_evaluation(bucket)))
            .collect(),
        associative_queries_with_two_or_more_hops,
        scope_leak_count,
        query_summaries,
    })
}

fn literal_path_summary(
    conn: &Connection,
    query: &golden::GoldenQuery,
    results: &[crate::memory::Memory],
    explain: &crate::retrieval::search::SearchExplain,
) -> Result<(Option<u8>, usize)> {
    let seed_ids = explain
        .channels
        .iter()
        .filter(|channel| channel.name == "fts" || channel.name == "vector")
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.memory_id))
        .take(32)
        .collect::<Vec<_>>();
    let outcome = crate::retrieval::graph::traverse_trusted_graph(
        conn,
        crate::retrieval::graph::GraphTraversalRequest {
            seed_memory_ids: &seed_ids,
            project: query.project.as_deref(),
            memory_type: query.memory_type.as_deref(),
            branch: query.branch.as_deref(),
            include_inactive: false,
            reference_time_epoch: chrono::Utc::now().timestamp(),
            limits: crate::retrieval::graph::GraphTraversalLimits::default(),
        },
    )?;
    let result_ids = results
        .iter()
        .map(|memory| memory.id)
        .collect::<BTreeSet<_>>();
    let graph_hits = outcome
        .hits
        .iter()
        .filter(|hit| result_ids.contains(&hit.memory_id))
        .collect::<Vec<_>>();
    Ok((
        graph_hits.iter().map(|hit| hit.hop_count).max(),
        graph_hits.len(),
    ))
}

fn ensure_required_slices(
    standard: &GraphDecisionArmReport,
    literal_graph: &GraphDecisionArmReport,
) -> Result<()> {
    if standard.associative_slice.scored_queries == 0
        || literal_graph.associative_slice.scored_queries == 0
    {
        bail!("graph decision eval requires scored associative queries in both arms");
    }
    Ok(())
}

fn build_deltas(
    standard: &GraphDecisionArmReport,
    literal_graph: &GraphDecisionArmReport,
) -> GraphDecisionDeltas {
    GraphDecisionDeltas {
        associative_recall_at_k: metric_delta(
            standard.associative_slice.metrics.as_ref(),
            literal_graph.associative_slice.metrics.as_ref(),
            |m| m.recall_at_k,
        ),
        associative_evidence_recall_at_k: metric_delta(
            standard.associative_slice.metrics.as_ref(),
            literal_graph.associative_slice.metrics.as_ref(),
            |m| m.evidence_recall_at_k,
        ),
        associative_ndcg_at_10: metric_delta(
            standard.associative_slice.metrics.as_ref(),
            literal_graph.associative_slice.metrics.as_ref(),
            |m| m.ndcg_at_10,
        ),
        non_associative_recall_at_k: metric_delta(
            standard.non_associative_slices.metrics.as_ref(),
            literal_graph.non_associative_slices.metrics.as_ref(),
            |m| m.recall_at_k,
        ),
        non_associative_evidence_recall_at_k: metric_delta(
            standard.non_associative_slices.metrics.as_ref(),
            literal_graph.non_associative_slices.metrics.as_ref(),
            |m| m.evidence_recall_at_k,
        ),
        non_associative_ndcg_at_10: metric_delta(
            standard.non_associative_slices.metrics.as_ref(),
            literal_graph.non_associative_slices.metrics.as_ref(),
            |m| m.ndcg_at_10,
        ),
        p95_latency_ms: literal_graph.overall.retrieval_latency_p95_ms
            - standard.overall.retrieval_latency_p95_ms,
    }
}

fn metric_delta(
    standard: Option<&MetricAverages>,
    candidate: Option<&MetricAverages>,
    value: impl Fn(&MetricAverages) -> f64,
) -> f64 {
    match (standard, candidate) {
        (Some(standard), Some(candidate)) => value(candidate) - value(standard),
        _ => 0.0,
    }
}

fn build_checks(
    standard: &GraphDecisionArmReport,
    literal_graph: &GraphDecisionArmReport,
    deltas: &GraphDecisionDeltas,
) -> GraphDecisionChecks {
    let associative_slice_present = standard.associative_slice.scored_queries > 0
        && literal_graph.associative_slice.scored_queries > 0;
    let literal_two_hop_observed = literal_graph.associative_queries_with_two_or_more_hops > 0;
    let benefit_threshold_met = deltas.associative_evidence_recall_at_k >= BENEFIT_THRESHOLD;
    let non_associative_zero_regression = non_associative_slices_not_lower(
        &standard.non_associative_by_slice,
        &literal_graph.non_associative_by_slice,
    );
    let zero_scope_leak = literal_graph.scope_leak_count == 0;
    let p95_latency_within_budget =
        literal_graph.overall.retrieval_latency_p95_ms <= LATENCY_BUDGET_P95_MS;
    let safe_to_wire_literal_graph = benefit_threshold_met
        && non_associative_zero_regression
        && zero_scope_leak
        && p95_latency_within_budget
        && literal_two_hop_observed;

    GraphDecisionChecks {
        associative_slice_present,
        literal_two_hop_observed,
        benefit_threshold_met,
        non_associative_zero_regression,
        zero_scope_leak,
        p95_latency_within_budget,
        safe_to_wire_literal_graph,
        all_checks_passed: associative_slice_present && safe_to_wire_literal_graph,
    }
}

fn metrics_not_lower(
    standard: Option<&MetricAverages>,
    candidate: Option<&MetricAverages>,
) -> bool {
    match (standard, candidate) {
        (Some(standard), Some(candidate)) => {
            candidate.hit_at_k + EPSILON >= standard.hit_at_k
                && candidate.mrr_at_10 + EPSILON >= standard.mrr_at_10
                && candidate.precision_at_k + EPSILON >= standard.precision_at_k
                && candidate.recall_at_k + EPSILON >= standard.recall_at_k
                && candidate.ndcg_at_10 + EPSILON >= standard.ndcg_at_10
                && candidate.evidence_recall_at_k + EPSILON >= standard.evidence_recall_at_k
        }
        (None, None) => true,
        _ => false,
    }
}

fn non_associative_slices_not_lower(
    standard: &BTreeMap<String, CategoryEvaluation>,
    candidate: &BTreeMap<String, CategoryEvaluation>,
) -> bool {
    standard.len() == candidate.len()
        && standard.iter().all(|(slice, standard)| {
            candidate.get(slice).is_some_and(|candidate| {
                metrics_not_lower(standard.metrics.as_ref(), candidate.metrics.as_ref())
                    && candidate.abstention_passed >= standard.abstention_passed
            })
        })
}

fn seed_fixture_graph_edges(conn: &Connection, dataset: &GoldenDataset) -> Result<()> {
    use crate::memory::graph_contract::{
        insert_graph_edge, GraphEdgeInput, GraphEdgeProvenance, GraphEdgeType, GraphNodeRef,
    };

    let (event_id, candidate_id, operation_id) = seed_graph_provenance(conn)?;
    let event_ids = [event_id];
    let provenance = GraphEdgeProvenance {
        source_event_ids: &event_ids,
        source_candidate_id: Some(candidate_id),
        source_operation_id: Some(operation_id),
        confidence: Some(1.0),
        reason: Some("pre-registered associative hop_path"),
    };
    let mut bridges = BTreeMap::<(String, String, String), GraphNodeRef>::new();
    let mut inserted = BTreeSet::<(String, i64, i64)>::new();
    for query in dataset
        .queries
        .iter()
        .filter(|query| query.slice_label() == "associative")
    {
        let hop = query
            .hop_path
            .as_ref()
            .with_context(|| format!("associative query {} missing hop_path", query.id))?;
        let project = query.project.as_deref().unwrap_or("");
        let key = (
            project.to_string(),
            hop.entity_type.clone(),
            hop.entity.clone(),
        );
        let bridge = if let Some(node) = bridges.get(&key) {
            *node
        } else {
            let node = create_graph_bridge(conn, project, &hop.entity_type, &hop.entity)?;
            bridges.insert(key, node);
            node
        };
        let edge_type = if hop.entity_type == "file_path" {
            GraphEdgeType::TouchesFile
        } else {
            GraphEdgeType::Mentions
        };
        for topic_key in [&hop.source, &hop.target] {
            let memory_id = conn
                .query_row(
                    "SELECT id FROM memories WHERE topic_key = ?1
                     AND (?2 IS NULL OR project = ?2)
                     AND (?3 IS NULL OR branch = ?3 OR branch IS NULL) LIMIT 1",
                    rusqlite::params![topic_key, query.project, query.branch],
                    |row| row.get(0),
                )
                .with_context(|| format!("resolve golden graph memory {topic_key}"))?;
            if inserted.insert((edge_type.as_str().to_string(), memory_id, bridge.id)) {
                insert_graph_edge(
                    conn,
                    &GraphEdgeInput {
                        edge_type,
                        from_node: GraphNodeRef::memory(memory_id)?,
                        to_node: bridge,
                        provenance,
                        valid_from_epoch: None,
                        valid_to_epoch: None,
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn seed_graph_provenance(conn: &Connection) -> Result<(i64, i64, i64)> {
    let now = 1_700_000_000_i64;
    let host_id: i64 =
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
         VALUES ('/tmp/remem-gh853-eval', 'origin', 'main', ?1, ?1)",
        [now],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/tmp/remem-gh853-eval', 'gh853-eval', ?2, ?2)",
        rusqlite::params![workspace_id, now],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch,
         last_seen_at_epoch, status) VALUES (?1, ?2, ?3, 'gh853-eval', ?4, ?4, 'active')",
        rusqlite::params![host_id, workspace_id, project_id, now],
    )?;
    let session_row_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id,
         session_id, event_id, event_type, content_hash, retention_class, created_at_epoch,
         inserted_at_epoch) VALUES (?1, ?2, ?3, ?4, 'gh853-eval', 'gh853-eval-event',
         'message', 'gh853-eval-hash', 'default', ?5, ?5)",
        rusqlite::params![host_id, workspace_id, project_id, session_row_id, now],
    )?;
    let event_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
         evidence_event_ids, confidence, risk_class, review_status, created_at_epoch,
         updated_at_epoch) VALUES (?1, 'project', 'decision', 'gh853-eval',
         'pre-registered graph fixture', ?2, 1.0, 'low', 'accepted', ?3, ?3)",
        rusqlite::params![project_id, format!("[{event_id}]"), now],
    )?;
    let candidate_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
         owner_scope, owner_key, memory_type, state_key, source_candidate_id, superseded_ids,
         conflicting_ids, confidence, reason, created_at_epoch) VALUES ('add', 'gh853-eval',
         'eval', 'memory_candidate', 'project', 'gh853-eval', 'decision', 'gh853-eval',
         ?1, '[]', '[]', 1.0, 'pre-registered graph fixture', ?2)",
        rusqlite::params![candidate_id, now],
    )?;
    Ok((event_id, candidate_id, conn.last_insert_rowid()))
}

fn create_graph_bridge(
    conn: &Connection,
    project: &str,
    entity_type: &str,
    entity: &str,
) -> Result<crate::memory::graph_contract::GraphNodeRef> {
    use crate::memory::graph_contract::GraphNodeRef;
    let now = 1_700_000_000_i64;
    if entity_type == "file_path" {
        conn.execute(
            "INSERT INTO graph_file_nodes(project_id, source_project, path,
             created_at_epoch, updated_at_epoch) VALUES (NULL, ?1, ?2, ?3, ?3)",
            rusqlite::params![project, entity, now],
        )?;
        return GraphNodeRef::file(conn.last_insert_rowid());
    }
    conn.execute(
        "INSERT OR IGNORE INTO entities(canonical_name, entity_type, mention_count,
         created_at_epoch) VALUES (?1, ?2, 1, ?3)",
        rusqlite::params![entity, entity_type, now],
    )?;
    let id = conn.query_row(
        "SELECT id FROM entities WHERE canonical_name = ?1 COLLATE NOCASE LIMIT 1",
        [entity],
        |row| row.get(0),
    )?;
    GraphNodeRef::entity(id)
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
            "associative evidence delta={:.3}, non-associative evidence delta={:.3}, literal-graph p95={:.2}ms",
            self.deltas.associative_evidence_recall_at_k,
            self.deltas.non_associative_evidence_recall_at_k,
            self.literal_graph.overall.retrieval_latency_p95_ms
        )?;
        writeln!(
            f,
            "checks: associative_slice_present={} literal_two_hop_observed={} benefit_threshold_met={} non_associative_zero_regression={} zero_scope_leak={} p95_latency_within_budget={} safe_to_wire_literal_graph={} all_checks_passed={}",
            self.checks.associative_slice_present,
            self.checks.literal_two_hop_observed,
            self.checks.benefit_threshold_met,
            self.checks.non_associative_zero_regression,
            self.checks.zero_scope_leak,
            self.checks.p95_latency_within_budget,
            self.checks.safe_to_wire_literal_graph,
            self.checks.all_checks_passed
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_decision_eval_wires_literal_graph_after_material_gain() -> Result<()> {
        let report = run_graph_decision_eval(GraphDecisionEvalOptions::default())?;
        assert_eq!(report.decision, GraphDecision::WireLiteralGraphTraversal);
        assert_eq!(
            report.evaluated_channel,
            EvaluatedGraphChannel::LiteralGraphEdges
        );
        assert!(report.graph_edges_evaluated);
        assert_eq!(
            report.graph_edges_retrieval_decision,
            GraphEdgesRetrievalDecision::WireProductionChannel
        );
        assert!(report.checks.all_checks_passed, "{report:#?}");
        assert!(report.checks.safe_to_wire_literal_graph);
        assert!(report.checks.benefit_threshold_met);
        assert!(report.checks.non_associative_zero_regression);
        assert!(report.checks.literal_two_hop_observed);
        assert!(report.checks.zero_scope_leak);
        assert!(report.deltas.associative_evidence_recall_at_k >= BENEFIT_THRESHOLD);
        let standard_non_associative = report
            .standard
            .non_associative_slices
            .metrics
            .as_ref()
            .context("standard non-associative metrics")?;
        let literal_non_associative = report
            .literal_graph
            .non_associative_slices
            .metrics
            .as_ref()
            .context("literal non-associative metrics")?;
        assert_eq!(
            literal_non_associative.precision_at_k,
            standard_non_associative.precision_at_k
        );
        assert!(non_associative_slices_not_lower(
            &report.standard.non_associative_by_slice,
            &report.literal_graph.non_associative_by_slice,
        ));
        let mut degraded = report.literal_graph.non_associative_by_slice.clone();
        let (slice, standard_slice) = report
            .standard
            .non_associative_by_slice
            .iter()
            .find(|(_, slice)| {
                slice
                    .metrics
                    .as_ref()
                    .is_some_and(|metrics| metrics.hit_at_k > 0.0)
            })
            .context("non-associative scored slice")?;
        degraded
            .get_mut(slice)
            .and_then(|slice| slice.metrics.as_mut())
            .context("candidate non-associative scored slice")?
            .hit_at_k = standard_slice
            .metrics
            .as_ref()
            .context("standard slice metrics")?
            .hit_at_k
            - 0.25;
        assert!(!non_associative_slices_not_lower(
            &report.standard.non_associative_by_slice,
            &degraded,
        ));
        Ok(())
    }

    #[test]
    fn graph_decision_eval_rejects_dataset_without_associative_slice() -> Result<()> {
        let mut dataset = golden::load_dataset(DEFAULT_DATASET_PATH)?;
        for query in &mut dataset.queries {
            if query.slice_label() == "associative" {
                query.slice = Some("paraphrase".to_string());
            }
        }

        let error = run_graph_decision_dataset(
            dataset,
            DEFAULT_DATASET_PATH.to_string(),
            GraphDecisionEvalOptions::default().k,
        )
        .expect_err("dataset without associative slice must fail the graph decision gate");

        assert!(error
            .to_string()
            .contains("requires scored associative queries"));
        Ok(())
    }
}
