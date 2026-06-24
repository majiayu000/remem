use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::eval::golden::{self, GoldenDataset, QueryStatus};
use crate::retrieval::search::SearchWeights;

const USAGE_SHADOW_WEIGHTS: [f64; 3] = [0.25, 0.75, 1.5];
const TOP_RESULT_CHANGE_SAMPLE_LIMIT: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct UsageShadowReport {
    pub default_usage_weight: f64,
    pub default_usage_weight_zero: bool,
    pub candidate_usage_weights: Vec<f64>,
    pub recommendation_boundary: &'static str,
    pub comparisons: Vec<UsageShadowComparison>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageShadowComparison {
    pub usage_weight: f64,
    pub baseline_scored_queries: usize,
    pub candidate_scored_queries: usize,
    pub scored_query_delta: isize,
    pub baseline_abstention_passed: usize,
    pub candidate_abstention_passed: usize,
    pub abstention_passed_delta: isize,
    pub top_result_changed_queries: usize,
    pub top_result_change_rate: f64,
    pub usage_channel_queries: usize,
    pub usage_channel_hits: usize,
    pub usage_scored_results: usize,
    pub max_usage_channel_score: f64,
    pub mean_usage_channel_score: f64,
    pub top_result_changes: Vec<UsageShadowTopResultChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageShadowTopResultChange {
    pub query_id: String,
    pub query: String,
    pub baseline_status: &'static str,
    pub candidate_status: &'static str,
    pub baseline_top_memory_id: Option<i64>,
    pub candidate_top_memory_id: Option<i64>,
    pub baseline_result_count: usize,
    pub candidate_result_count: usize,
    pub candidate_top_usage_score: f64,
}

struct UsageShadowRun {
    scored_queries: usize,
    abstention_passed: usize,
    queries: Vec<UsageShadowQuerySnapshot>,
}

struct UsageShadowQuerySnapshot {
    id: String,
    query: String,
    status: QueryStatus,
    top_memory_id: Option<i64>,
    result_count: usize,
    usage_scores_by_memory_id: BTreeMap<i64, f64>,
}

pub(super) fn build_usage_shadow_report(
    conn: &Connection,
    dataset: &GoldenDataset,
    k: usize,
) -> Result<UsageShadowReport> {
    let default_weights = SearchWeights::default();
    let baseline = run_usage_shadow_candidate(conn, dataset, k, default_weights)?;
    let mut comparisons = Vec::with_capacity(USAGE_SHADOW_WEIGHTS.len());
    for usage_weight in USAGE_SHADOW_WEIGHTS {
        let candidate_weights = SearchWeights {
            usage: usage_weight,
            ..default_weights
        };
        let candidate = run_usage_shadow_candidate(conn, dataset, k, candidate_weights)?;
        comparisons.push(compare_usage_shadow_runs(
            usage_weight,
            &baseline,
            &candidate,
        ));
    }

    Ok(UsageShadowReport {
        default_usage_weight: default_weights.usage,
        default_usage_weight_zero: default_weights.usage == 0.0,
        candidate_usage_weights: USAGE_SHADOW_WEIGHTS.to_vec(),
        recommendation_boundary:
            "report_only_no_default_change_without_eval_gates_and_coding_agent_ab",
        comparisons,
    })
}

fn run_usage_shadow_candidate(
    conn: &Connection,
    dataset: &GoldenDataset,
    k: usize,
    weights: SearchWeights,
) -> Result<UsageShadowRun> {
    let fetch_limit = k.max(10) as i64;
    let mut scored_queries = 0usize;
    let mut abstention_passed = 0usize;
    let mut query_snapshots = Vec::with_capacity(dataset.queries.len());

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
        if query.expects_abstention() {
            if evaluation.status == QueryStatus::Pass {
                abstention_passed += 1;
            }
        } else if evaluation.metrics.is_some() {
            scored_queries += 1;
        }

        let result_ids = results.iter().map(|memory| memory.id).collect::<Vec<_>>();
        let usage_scores_by_memory_id = if weights.usage > 0.0 {
            let candidate_ids = usage_shadow_candidate_ids(conn, query, fetch_limit)?;
            crate::retrieval::search::usage_hits_for_retrieved_candidates(
                conn,
                &candidate_ids,
                weights,
            )?
            .into_iter()
            .map(|hit| (hit.id, hit.normalized_score))
            .collect()
        } else {
            BTreeMap::new()
        };
        query_snapshots.push(UsageShadowQuerySnapshot {
            id: query.id.clone(),
            query: query.query.clone(),
            status: evaluation.status,
            top_memory_id: result_ids.first().copied(),
            result_count: results.len(),
            usage_scores_by_memory_id,
        });
    }

    Ok(UsageShadowRun {
        scored_queries,
        abstention_passed,
        queries: query_snapshots,
    })
}

fn usage_shadow_candidate_ids(
    conn: &Connection,
    query: &golden::GoldenQuery,
    fetch_limit: i64,
) -> Result<Vec<i64>> {
    let (_results, explain) = crate::retrieval::search::search_with_branch_explain(
        conn,
        Some(&query.query),
        query.project.as_deref(),
        query.memory_type.as_deref(),
        fetch_limit,
        0,
        false,
        query.branch.as_deref(),
    )?;
    let mut ids = explain
        .into_iter()
        .flat_map(|explain| explain.channels)
        .filter(|channel| channel.enabled && channel.name != "usage")
        .flat_map(|channel| channel.hits.into_iter().map(|hit| hit.memory_id))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn compare_usage_shadow_runs(
    usage_weight: f64,
    baseline: &UsageShadowRun,
    candidate: &UsageShadowRun,
) -> UsageShadowComparison {
    let mut top_result_changed_queries = 0usize;
    let mut top_result_changes = Vec::new();
    let mut usage_channel_queries = 0usize;
    let mut usage_channel_hits = 0usize;
    let mut usage_scored_results = 0usize;
    let mut usage_score_sum = 0.0;
    let mut max_usage_channel_score = 0.0_f64;

    for (baseline_query, candidate_query) in baseline.queries.iter().zip(&candidate.queries) {
        if !candidate_query.usage_scores_by_memory_id.is_empty() {
            usage_channel_queries += 1;
            usage_channel_hits += candidate_query.usage_scores_by_memory_id.len();
            usage_scored_results += candidate_query.usage_scores_by_memory_id.len();
            for score in candidate_query.usage_scores_by_memory_id.values() {
                usage_score_sum += *score;
                max_usage_channel_score = max_usage_channel_score.max(*score);
            }
        }

        if baseline_query.top_memory_id != candidate_query.top_memory_id {
            top_result_changed_queries += 1;
            if top_result_changes.len() < TOP_RESULT_CHANGE_SAMPLE_LIMIT {
                let candidate_top_usage_score = candidate_query
                    .top_memory_id
                    .and_then(|id| candidate_query.usage_scores_by_memory_id.get(&id).copied())
                    .unwrap_or(0.0);
                top_result_changes.push(UsageShadowTopResultChange {
                    query_id: baseline_query.id.clone(),
                    query: baseline_query.query.clone(),
                    baseline_status: baseline_query.status.label(),
                    candidate_status: candidate_query.status.label(),
                    baseline_top_memory_id: baseline_query.top_memory_id,
                    candidate_top_memory_id: candidate_query.top_memory_id,
                    baseline_result_count: baseline_query.result_count,
                    candidate_result_count: candidate_query.result_count,
                    candidate_top_usage_score,
                });
            }
        }
    }

    UsageShadowComparison {
        usage_weight,
        baseline_scored_queries: baseline.scored_queries,
        candidate_scored_queries: candidate.scored_queries,
        scored_query_delta: usize_delta(candidate.scored_queries, baseline.scored_queries),
        baseline_abstention_passed: baseline.abstention_passed,
        candidate_abstention_passed: candidate.abstention_passed,
        abstention_passed_delta: usize_delta(
            candidate.abstention_passed,
            baseline.abstention_passed,
        ),
        top_result_changed_queries,
        top_result_change_rate: rate(top_result_changed_queries, baseline.queries.len()),
        usage_channel_queries,
        usage_channel_hits,
        usage_scored_results,
        max_usage_channel_score,
        mean_usage_channel_score: if usage_scored_results == 0 {
            0.0
        } else {
            usage_score_sum / usage_scored_results as f64
        },
        top_result_changes,
    }
}

fn rate(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn usize_delta(candidate: usize, baseline: usize) -> isize {
    candidate as isize - baseline as isize
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use rusqlite::Connection;

    use super::*;
    use crate::eval::golden::{EvidenceRef, GoldenMemory, GoldenQuery};

    #[test]
    fn usage_shadow_reports_usage_scores_without_changing_default_weight() -> Result<()> {
        let dataset = GoldenDataset {
            version: Some("usage-shadow-test".to_string()),
            description: None,
            corpus: vec![
                GoldenMemory {
                    project: "/repo".to_string(),
                    topic_key: Some("sqlite-timeout-old".to_string()),
                    title: "SQLite timeout old path".to_string(),
                    content: "SQLite timeout fix should update busy_timeout.".to_string(),
                    memory_type: "decision".to_string(),
                    branch: None,
                    scope: "project".to_string(),
                    status: "active".to_string(),
                    files: None,
                    created_at_epoch: Some(100),
                    access_count: Some(1),
                    last_accessed_epoch: Some(100),
                },
                GoldenMemory {
                    project: "/repo".to_string(),
                    topic_key: Some("sqlite-timeout-proven".to_string()),
                    title: "SQLite timeout proven path".to_string(),
                    content: "SQLite timeout fix should update busy_timeout.".to_string(),
                    memory_type: "decision".to_string(),
                    branch: None,
                    scope: "project".to_string(),
                    status: "active".to_string(),
                    files: None,
                    created_at_epoch: Some(101),
                    access_count: Some(50),
                    last_accessed_epoch: Some(chrono::Utc::now().timestamp()),
                },
            ],
            queries: vec![GoldenQuery {
                id: "q1".to_string(),
                query: "SQLite timeout busy_timeout".to_string(),
                category: "retrieval".to_string(),
                slice: Some("usage-shadow".to_string()),
                project: Some("/repo".to_string()),
                branch: None,
                memory_type: None,
                relevant_ids: vec![],
                evidence_refs: vec![EvidenceRef {
                    topic_key: Some("sqlite-timeout-proven".to_string()),
                    ..EvidenceRef::default()
                }],
                expect_abstain: false,
                false_premise: false,
                notes: None,
            }],
        };
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        golden::run::seed_fixture_corpus(&conn, &dataset.corpus)?;

        let report = build_usage_shadow_report(&conn, &dataset, 5)?;

        assert!(report.default_usage_weight_zero);
        let strongest = report
            .comparisons
            .iter()
            .max_by(|left, right| left.usage_weight.total_cmp(&right.usage_weight))
            .context("usage shadow comparisons should be present")?;
        assert!(strongest.usage_channel_hits > 0);
        assert!(strongest.max_usage_channel_score > 0.0);
        assert_eq!(
            strongest.baseline_scored_queries,
            strongest.candidate_scored_queries
        );
        Ok(())
    }

    #[test]
    fn usage_shadow_counts_usage_scores_before_final_pagination() -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let corpus = (0..12)
            .map(|index| GoldenMemory {
                project: "/repo".to_string(),
                topic_key: Some(format!("sqlite-timeout-{index:02}")),
                title: format!("SQLite timeout candidate {index:02}"),
                content: "SQLite timeout fix should update busy_timeout in the connection setup."
                    .to_string(),
                memory_type: "decision".to_string(),
                branch: None,
                scope: "project".to_string(),
                status: "active".to_string(),
                files: None,
                created_at_epoch: Some(100 + index as i64),
                access_count: Some((index + 1) as i64),
                last_accessed_epoch: Some(now),
            })
            .collect();
        let dataset = GoldenDataset {
            version: Some("usage-shadow-pre-pagination-test".to_string()),
            description: None,
            corpus,
            queries: vec![GoldenQuery {
                id: "q1".to_string(),
                query: "SQLite timeout busy_timeout".to_string(),
                category: "retrieval".to_string(),
                slice: Some("usage-shadow".to_string()),
                project: Some("/repo".to_string()),
                branch: None,
                memory_type: None,
                relevant_ids: vec![],
                evidence_refs: vec![EvidenceRef {
                    topic_key: Some("sqlite-timeout-11".to_string()),
                    ..EvidenceRef::default()
                }],
                expect_abstain: false,
                false_premise: false,
                notes: None,
            }],
        };
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        golden::run::seed_fixture_corpus(&conn, &dataset.corpus)?;

        let report = build_usage_shadow_report(&conn, &dataset, 5)?;

        let strongest = report
            .comparisons
            .iter()
            .max_by(|left, right| left.usage_weight.total_cmp(&right.usage_weight))
            .context("usage shadow comparisons should be present")?;
        assert_eq!(strongest.usage_channel_queries, 1);
        assert_eq!(strongest.usage_channel_hits, 12);
        assert_eq!(strongest.usage_scored_results, 12);
        Ok(())
    }
}
