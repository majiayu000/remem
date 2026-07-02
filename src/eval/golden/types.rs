use std::collections::BTreeMap;

use crate::memory::Memory;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GoldenDataset {
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub corpus: Vec<GoldenMemory>,
    #[serde(default)]
    pub queries: Vec<GoldenQuery>,
}

impl GoldenDataset {
    pub fn has_fixture_corpus(&self) -> bool {
        !self.corpus.is_empty()
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GoldenMemory {
    pub project: String,
    #[serde(default)]
    pub topic_key: Option<String>,
    pub title: String,
    #[serde(alias = "text")]
    pub content: String,
    pub memory_type: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub files: Option<String>,
    #[serde(default)]
    pub created_at_epoch: Option<i64>,
    #[serde(default)]
    pub access_count: Option<i64>,
    #[serde(default)]
    pub last_accessed_epoch: Option<i64>,
}

fn default_scope() -> String {
    "project".to_string()
}

fn default_status() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GoldenQuery {
    pub id: String,
    pub query: String,
    pub category: String,
    #[serde(default)]
    pub slice: Option<String>,
    #[serde(default)]
    pub hop_path: Option<GoldenHopPath>,
    pub project: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub relevant_ids: Vec<i64>,
    #[serde(default, alias = "expected_refs")]
    pub evidence_refs: Vec<EvidenceRef>,
    #[serde(default)]
    pub expect_abstain: bool,
    #[serde(default)]
    pub false_premise: bool,
    pub notes: Option<String>,
}

impl GoldenQuery {
    pub fn expects_abstention(&self) -> bool {
        self.expect_abstain || self.false_premise
    }

    pub fn slice_label(&self) -> &str {
        self.slice
            .as_deref()
            .filter(|slice| !slice.trim().is_empty())
            .unwrap_or(&self.category)
    }

    pub fn expected_refs(&self) -> Vec<EvidenceRef> {
        let mut refs = self.evidence_refs.clone();
        refs.extend(self.relevant_ids.iter().map(|id| EvidenceRef {
            memory_id: Some(*id),
            ..EvidenceRef::default()
        }));
        refs
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GoldenHopPath {
    pub source: String,
    pub entity_type: String,
    pub entity: String,
    pub target: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct EvidenceRef {
    pub memory_id: Option<i64>,
    pub topic_key: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub memory_type: Option<String>,
    pub scope: Option<String>,
    pub title_contains: Option<String>,
    pub text_contains: Option<String>,
}

impl EvidenceRef {
    pub fn has_match_criteria(&self) -> bool {
        self.memory_id.is_some()
            || self.topic_key.is_some()
            || self.project.is_some()
            || self.branch.is_some()
            || self.memory_type.is_some()
            || self.scope.is_some()
            || self.title_contains.is_some()
            || self.text_contains.is_some()
    }

    pub fn matches(&self, memory: &Memory) -> bool {
        if !self.has_match_criteria() {
            return false;
        }
        if let Some(memory_id) = self.memory_id {
            if memory.id != memory_id {
                return false;
            }
        }
        if let Some(project) = self.project.as_deref() {
            if !crate::project_id::project_matches(Some(&memory.project), project) {
                return false;
            }
        }
        if let Some(branch) = self.branch.as_deref() {
            if memory.branch.as_deref() != Some(branch) {
                return false;
            }
        }
        if let Some(topic_key) = self.topic_key.as_deref() {
            if memory.topic_key.as_deref() != Some(topic_key) {
                return false;
            }
        }
        if let Some(memory_type) = self.memory_type.as_deref() {
            if memory.memory_type != memory_type {
                return false;
            }
        }
        if let Some(scope) = self.scope.as_deref() {
            if memory.scope != scope {
                return false;
            }
        }
        if let Some(needle) = self.title_contains.as_deref() {
            if !contains_case_insensitive(&memory.title, needle) {
                return false;
            }
        }
        if let Some(needle) = self.text_contains.as_deref() {
            if !contains_case_insensitive(&memory.text, needle) {
                return false;
            }
        }
        true
    }
}

pub(super) fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GoldenEvalReport {
    pub evaluation_layers: EvaluationLayers,
    pub version: Option<String>,
    pub description: Option<String>,
    pub k: usize,
    pub rank_k: usize,
    pub total_queries: usize,
    pub scored_queries: usize,
    pub skipped_queries: usize,
    pub abstention_queries: usize,
    pub abstention_passed: usize,
    pub overall: Option<MetricAverages>,
    pub by_slice: BTreeMap<String, CategoryEvaluation>,
    pub by_category: BTreeMap<String, CategoryEvaluation>,
    pub queries: Vec<QueryEvaluation>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CategoryEvaluation {
    pub total_queries: usize,
    pub scored_queries: usize,
    pub abstention_queries: usize,
    pub abstention_passed: usize,
    pub query_tokens_per_query: f64,
    pub retrieval_latency_p50_ms: f64,
    pub retrieval_latency_p95_ms: f64,
    pub metrics: Option<MetricAverages>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct QueryEvaluation {
    pub id: String,
    pub query: String,
    pub category: String,
    pub slice: String,
    pub status: QueryStatus,
    pub result_count: usize,
    pub retrieved_ids: Vec<i64>,
    pub expected_relevant_ids: Vec<i64>,
    pub missing_relevant_ids: Vec<i64>,
    pub missing_evidence_refs: Vec<EvidenceRef>,
    pub matched_refs: usize,
    pub expected_refs: usize,
    pub query_tokens: usize,
    pub retrieval_latency_ms: f64,
    pub metrics: Option<QueryMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum QueryStatus {
    #[serde(rename = "HIT")]
    Hit,
    #[serde(rename = "MISS")]
    Miss,
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "SKIP")]
    Skip,
}

impl QueryStatus {
    pub fn label(self) -> &'static str {
        match self {
            QueryStatus::Hit => "HIT",
            QueryStatus::Miss => "MISS",
            QueryStatus::Pass => "PASS",
            QueryStatus::Fail => "FAIL",
            QueryStatus::Skip => "SKIP",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EvaluationLayers {
    pub retrieval: LayerStatus,
    pub answer_generation: LayerStatus,
    pub llm_judge: LayerStatus,
}

impl EvaluationLayers {
    pub fn deterministic_retrieval_only() -> Self {
        Self {
            retrieval: LayerStatus {
                status: "deterministic",
                description:
                    "fixed golden retrieval metrics: Hit@K, Recall@K, MRR, nDCG, evidence recall",
            },
            answer_generation: LayerStatus {
                status: "not_run",
                description:
                    "answer generation is intentionally excluded from golden retrieval eval",
            },
            llm_judge: LayerStatus {
                status: "not_run",
                description: "LLM judging is intentionally excluded from deterministic golden eval",
            },
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LayerStatus {
    pub status: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct QueryMetrics {
    pub hit_at_k: f64,
    pub mrr_at_10: f64,
    pub precision_at_k: f64,
    pub recall_at_k: f64,
    pub ndcg_at_10: f64,
    pub evidence_recall_at_k: f64,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MetricAverages {
    pub count: usize,
    pub hit_at_k: f64,
    pub mrr_at_10: f64,
    pub precision_at_k: f64,
    pub recall_at_k: f64,
    pub ndcg_at_10: f64,
    pub evidence_recall_at_k: f64,
}

#[derive(Debug, Default)]
pub(super) struct MetricSums {
    count: usize,
    hit_at_k: f64,
    mrr_at_10: f64,
    precision_at_k: f64,
    recall_at_k: f64,
    ndcg_at_10: f64,
    evidence_recall_at_k: f64,
}

impl MetricSums {
    pub(super) fn add(&mut self, metrics: &QueryMetrics) {
        self.count += 1;
        self.hit_at_k += metrics.hit_at_k;
        self.mrr_at_10 += metrics.mrr_at_10;
        self.precision_at_k += metrics.precision_at_k;
        self.recall_at_k += metrics.recall_at_k;
        self.ndcg_at_10 += metrics.ndcg_at_10;
        self.evidence_recall_at_k += metrics.evidence_recall_at_k;
    }

    pub(super) fn averages(&self) -> Option<MetricAverages> {
        if self.count == 0 {
            return None;
        }
        let n = self.count as f64;
        Some(MetricAverages {
            count: self.count,
            hit_at_k: self.hit_at_k / n,
            mrr_at_10: self.mrr_at_10 / n,
            precision_at_k: self.precision_at_k / n,
            recall_at_k: self.recall_at_k / n,
            ndcg_at_10: self.ndcg_at_10 / n,
            evidence_recall_at_k: self.evidence_recall_at_k / n,
        })
    }
}
