use std::collections::BTreeMap;
use std::fmt::{Display, Formatter, Result as FmtResult};

use anyhow::{ensure, Context, Result};
use serde::Serialize;

use crate::eval::golden::{self, GoldenDataset, GoldenMemory, GoldenQuery, MetricAverages};

pub const DEFAULT_DATASET_PATH: &str = "eval/golden.json";
pub const DEFAULT_REPORT_PATH: &str = "eval/associative-multihop/baseline.json";

const REPORT_VERSION: &str = "2026-07-02";
const ASSOCIATIVE_SLICE: &str = "associative";

#[derive(Debug, Clone)]
pub struct AssociativeBaselineOptions {
    pub dataset_path: String,
    pub k: usize,
}

impl Default for AssociativeBaselineOptions {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            k: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociativeBaselineReport {
    pub version: &'static str,
    pub dataset_path: String,
    pub slice: &'static str,
    pub k: usize,
    pub query_count: usize,
    pub entity_type_counts: BTreeMap<String, usize>,
    pub max_query_target_shared_tokens: usize,
    pub baseline_fused: AssociativeFusedMetrics,
    pub headroom: AssociativeHeadroom,
    pub fixtures: Vec<AssociativeFixtureSummary>,
    pub omitted_followups: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociativeFusedMetrics {
    pub scored_queries: usize,
    pub hit_at_k: f64,
    pub mrr_at_10: f64,
    pub precision_at_k: f64,
    pub recall_at_k: f64,
    pub ndcg_at_10: f64,
    pub evidence_recall_at_k: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociativeHeadroom {
    pub hit_at_k: f64,
    pub recall_at_k: f64,
    pub ndcg_at_10: f64,
    pub evidence_recall_at_k: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociativeFixtureSummary {
    pub id: String,
    pub entity_type: String,
    pub source: String,
    pub target: String,
    pub shared_tokens: Vec<String>,
}

pub fn run_associative_baseline(
    options: AssociativeBaselineOptions,
) -> Result<AssociativeBaselineReport> {
    let dataset = golden::load_dataset(&options.dataset_path)?;
    run_associative_baseline_for_dataset(options, dataset)
}

pub(in crate::eval) fn run_associative_baseline_for_dataset(
    options: AssociativeBaselineOptions,
    dataset: GoldenDataset,
) -> Result<AssociativeBaselineReport> {
    ensure!(
        dataset.has_fixture_corpus(),
        "associative baseline requires a fixture-backed golden dataset"
    );
    let associative_queries = associative_queries(&dataset);
    ensure!(
        associative_queries.len() >= 15,
        "associative baseline requires at least 15 fixtures, found {}",
        associative_queries.len()
    );

    let k = options.k.max(1);
    let entity_type_counts = entity_type_counts(&associative_queries)?;
    let fixtures = fixture_summaries(&dataset, &associative_queries)?;
    let max_query_target_shared_tokens = fixtures
        .iter()
        .map(|fixture| fixture.shared_tokens.len())
        .max()
        .unwrap_or(0);
    let filtered_dataset = GoldenDataset {
        version: dataset.version.clone(),
        description: dataset.description.clone(),
        corpus: dataset.corpus.clone(),
        queries: associative_queries,
    };
    let golden_report = golden::evaluate_dataset_with_fixture_corpus(&filtered_dataset, k)
        .context("run associative baseline golden eval")?;
    let slice = golden_report
        .by_slice
        .get(ASSOCIATIVE_SLICE)
        .context("associative baseline report missing associative slice")?;
    let empty = MetricAverages::default();
    let metrics = slice.metrics.as_ref().unwrap_or(&empty);
    let baseline_fused = AssociativeFusedMetrics::from(metrics);
    let headroom = AssociativeHeadroom::from(&baseline_fused);

    Ok(AssociativeBaselineReport {
        version: REPORT_VERSION,
        dataset_path: options.dataset_path,
        slice: ASSOCIATIVE_SLICE,
        k,
        query_count: filtered_dataset.queries.len(),
        entity_type_counts,
        max_query_target_shared_tokens,
        baseline_fused,
        headroom,
        fixtures,
        omitted_followups: vec![
            "per_channel_attribution",
            "entity_bfs_proxy_delta",
            "literal_graph_edges_traversal",
            "adr_decision_followup",
            "production_retrieval_wiring",
        ],
    })
}

fn associative_queries(dataset: &GoldenDataset) -> Vec<GoldenQuery> {
    dataset
        .queries
        .iter()
        .filter(|query| query.slice_label() == ASSOCIATIVE_SLICE)
        .cloned()
        .collect()
}

fn entity_type_counts(queries: &[GoldenQuery]) -> Result<BTreeMap<String, usize>> {
    let mut counts = BTreeMap::new();
    for query in queries {
        let hop_path = query
            .hop_path
            .as_ref()
            .with_context(|| format!("associative query {} missing hop_path", query.id))?;
        *counts.entry(hop_path.entity_type.clone()).or_insert(0) += 1;
    }
    Ok(counts)
}

fn fixture_summaries(
    dataset: &GoldenDataset,
    queries: &[GoldenQuery],
) -> Result<Vec<AssociativeFixtureSummary>> {
    queries
        .iter()
        .map(|query| {
            let hop_path = query
                .hop_path
                .as_ref()
                .with_context(|| format!("associative query {} missing hop_path", query.id))?;
            let target =
                find_target_memory(dataset, query, &hop_path.target).with_context(|| {
                    format!(
                        "associative query {} target {} missing from corpus",
                        query.id, hop_path.target
                    )
                })?;
            Ok(AssociativeFixtureSummary {
                id: query.id.clone(),
                entity_type: hop_path.entity_type.clone(),
                source: hop_path.source.clone(),
                target: hop_path.target.clone(),
                shared_tokens: golden::validation::query_target_shared_tokens(
                    &query.query,
                    &target.content,
                )
                .into_iter()
                .collect(),
            })
        })
        .collect()
}

fn find_target_memory<'a>(
    dataset: &'a GoldenDataset,
    query: &GoldenQuery,
    target_topic_key: &str,
) -> Option<&'a GoldenMemory> {
    dataset.corpus.iter().find(|memory| {
        memory.status == "active"
            && memory.topic_key.as_deref() == Some(target_topic_key)
            && query_matches_memory_filter(query, memory)
    })
}

fn query_matches_memory_filter(query: &GoldenQuery, memory: &GoldenMemory) -> bool {
    if let Some(project) = query.project.as_deref() {
        if !crate::project_id::project_matches(Some(&memory.project), project) {
            return false;
        }
    }
    if let Some(branch) = query.branch.as_deref() {
        if memory.branch.as_deref() != Some(branch) {
            return false;
        }
    }
    if let Some(memory_type) = query.memory_type.as_deref() {
        if memory.memory_type != memory_type {
            return false;
        }
    }
    true
}

impl From<&MetricAverages> for AssociativeFusedMetrics {
    fn from(metrics: &MetricAverages) -> Self {
        Self {
            scored_queries: metrics.count,
            hit_at_k: metrics.hit_at_k,
            mrr_at_10: metrics.mrr_at_10,
            precision_at_k: metrics.precision_at_k,
            recall_at_k: metrics.recall_at_k,
            ndcg_at_10: metrics.ndcg_at_10,
            evidence_recall_at_k: metrics.evidence_recall_at_k,
        }
    }
}

impl From<&AssociativeFusedMetrics> for AssociativeHeadroom {
    fn from(metrics: &AssociativeFusedMetrics) -> Self {
        Self {
            hit_at_k: 1.0 - metrics.hit_at_k,
            recall_at_k: 1.0 - metrics.recall_at_k,
            ndcg_at_10: 1.0 - metrics.ndcg_at_10,
            evidence_recall_at_k: 1.0 - metrics.evidence_recall_at_k,
        }
    }
}

impl Display for AssociativeBaselineReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(
            f,
            "remem eval-associative-baseline - slice={} k={}",
            self.slice, self.k
        )?;
        writeln!(f, "dataset: {}", self.dataset_path)?;
        writeln!(
            f,
            "fixtures: {} max_query_target_shared_tokens={}",
            self.query_count, self.max_query_target_shared_tokens
        )?;
        writeln!(
            f,
            "baseline fused: hit@{}={:.3} recall@{}={:.3} nDCG@10={:.3} evidence@{}={:.3}",
            self.k,
            self.baseline_fused.hit_at_k,
            self.k,
            self.baseline_fused.recall_at_k,
            self.baseline_fused.ndcg_at_10,
            self.k,
            self.baseline_fused.evidence_recall_at_k
        )?;
        writeln!(
            f,
            "headroom: hit@{}={:.3} recall@{}={:.3} nDCG@10={:.3} evidence@{}={:.3}",
            self.k,
            self.headroom.hit_at_k,
            self.k,
            self.headroom.recall_at_k,
            self.headroom.ndcg_at_10,
            self.k,
            self.headroom.evidence_recall_at_k
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::eval::golden::{EvidenceRef, GoldenHopPath};

    fn test_memory(topic_key: &str, title: &str, content: &str, memory_type: &str) -> GoldenMemory {
        GoldenMemory {
            project: "/repo".to_string(),
            topic_key: Some(topic_key.to_string()),
            title: title.to_string(),
            content: content.to_string(),
            memory_type: memory_type.to_string(),
            branch: Some("main".to_string()),
            scope: "project".to_string(),
            status: "active".to_string(),
            files: None,
            created_at_epoch: None,
            access_count: None,
            last_accessed_epoch: None,
        }
    }

    #[test]
    fn checked_in_associative_baseline_contract_has_fixture_headroom() -> Result<()> {
        let report = run_associative_baseline(Default::default())?;

        assert_eq!(report.query_count, 15);
        assert_eq!(
            report.entity_type_counts,
            BTreeMap::from([
                ("crate".to_string(), 4),
                ("error_signature".to_string(), 4),
                ("file_path".to_string(), 4),
                ("issue_number".to_string(), 3),
            ])
        );
        assert_eq!(report.max_query_target_shared_tokens, 0);
        assert!(report.baseline_fused.recall_at_k < 1.0);
        assert!(report.headroom.recall_at_k > 0.0);
        assert!(report
            .omitted_followups
            .contains(&"literal_graph_edges_traversal"));
        Ok(())
    }

    #[test]
    fn checked_in_associative_baseline_json_matches_generated_report() -> Result<()> {
        let report = run_associative_baseline(Default::default())?;
        let committed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(DEFAULT_REPORT_PATH)?)?;

        assert_eq!(committed["version"], report.version);
        assert_eq!(committed["dataset_path"], report.dataset_path);
        assert_eq!(committed["slice"], report.slice);
        assert_eq!(committed["k"], report.k);
        assert_eq!(committed["query_count"], report.query_count);
        assert_eq!(
            committed["max_query_target_shared_tokens"],
            report.max_query_target_shared_tokens
        );
        assert_eq!(
            committed["entity_type_counts"],
            serde_json::to_value(&report.entity_type_counts)?
        );
        assert_eq!(
            committed["baseline_fused"],
            serde_json::to_value(&report.baseline_fused)?
        );
        assert_eq!(
            committed["headroom"],
            serde_json::to_value(&report.headroom)?
        );
        Ok(())
    }

    #[test]
    fn associative_validation_rejects_query_target_token_leak() -> Result<()> {
        let dataset = GoldenDataset {
            version: Some("associative-test".to_string()),
            description: None,
            corpus: vec![
                test_memory(
                    "bridge",
                    "Bridge",
                    "Bridge carries entity src/alpha.rs",
                    "discovery",
                ),
                test_memory(
                    "target",
                    "Target",
                    "src/alpha.rs stores leaked answer token.",
                    "decision",
                ),
            ],
            queries: vec![GoldenQuery {
                id: "leaky".to_string(),
                query: "which leaked result".to_string(),
                category: "multi_hop".to_string(),
                slice: Some("associative".to_string()),
                hop_path: Some(GoldenHopPath {
                    source: "bridge".to_string(),
                    entity_type: "file_path".to_string(),
                    entity: "src/alpha.rs".to_string(),
                    target: "target".to_string(),
                }),
                project: Some("/repo".to_string()),
                branch: Some("main".to_string()),
                memory_type: None,
                relevant_ids: vec![],
                evidence_refs: vec![EvidenceRef {
                    topic_key: Some("target".to_string()),
                    ..EvidenceRef::default()
                }],
                expect_abstain: false,
                false_premise: false,
                notes: None,
            }],
        };

        let error = golden::evaluate_dataset_with_fixture_corpus(&dataset, 5)
            .expect_err("token leak should be rejected");
        assert!(error.to_string().contains("token overlap"));
        Ok(())
    }
}
