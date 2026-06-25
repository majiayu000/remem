use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use super::types::{
    MemoryBenchCondition, MemoryBenchDiagnosisOutcome, MemoryBenchFailureDecomposition,
    MemoryBenchPerformanceMetrics, MemoryBenchPerformanceSummary, MemoryBenchPolicyOutcome,
    MemoryBenchRunOutcome, MemoryBenchTask,
};

pub(super) fn score_policy(
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
    retrieved_event_ids: &[String],
    abstained: bool,
) -> MemoryBenchPolicyOutcome {
    let Some(policy) = &task.policy else {
        return MemoryBenchPolicyOutcome::default();
    };
    let storage_condition = stores_memory_condition(condition);
    let active_claim_count = if storage_condition
        && task
            .evidence
            .iter()
            .any(|evidence| evidence.retention_allowed)
    {
        1
    } else {
        0
    };
    let candidate_count = 0;
    let summary_input_count = if active_claim_count > 0 { 1 } else { 0 };

    let non_retention_applicable = storage_condition && policy.non_retention_required;
    let non_retention_leaked = non_retention_applicable
        && (active_claim_count > 0
            || candidate_count > 0
            || summary_input_count > 0
            || retrieved_event_ids
                .iter()
                .any(|event_id| !evidence_retention_allowed(task, event_id)));
    let false_block_applicable = storage_condition && policy.explicit_approval;
    let false_blocked =
        false_block_applicable && (active_claim_count < policy.expected_active_claims || abstained);
    let suppression_applicable = storage_condition && policy.suppression_expected;
    let suppression_obeyed = suppression_applicable
        && active_claim_count == 0
        && candidate_count == 0
        && summary_input_count == 0
        && retrieved_event_ids.is_empty();
    let sensitive_restricted_applicable = storage_condition && policy.sensitive_or_restricted;
    let sensitive_restricted_default_excluded = sensitive_restricted_applicable
        && active_claim_count == 0
        && candidate_count == 0
        && summary_input_count == 0;
    let policy_abstention_applicable =
        policy.expected_policy_abstention || policy.explicit_approval || storage_condition;
    let policy_abstention_correct = if policy.expected_policy_abstention {
        abstained
    } else if false_block_applicable {
        !abstained
    } else {
        true
    };

    let mut policy_failure_count = 0;
    if non_retention_leaked {
        policy_failure_count += 1;
    }
    if false_blocked {
        policy_failure_count += 1;
    }
    if suppression_applicable && !suppression_obeyed {
        policy_failure_count += 1;
    }
    if sensitive_restricted_applicable && !sensitive_restricted_default_excluded {
        policy_failure_count += 1;
    }
    if policy_abstention_applicable && !policy_abstention_correct {
        policy_failure_count += 1;
    }
    if storage_condition {
        if active_claim_count != policy.expected_active_claims {
            policy_failure_count += 1;
        }
        if candidate_count != policy.expected_candidates {
            policy_failure_count += 1;
        }
        if summary_input_count != policy.expected_summary_inputs {
            policy_failure_count += 1;
        }
    }

    MemoryBenchPolicyOutcome {
        active_claim_count,
        candidate_count,
        summary_input_count,
        non_retention_applicable,
        non_retention_leaked,
        false_block_applicable,
        false_blocked,
        suppression_applicable,
        suppression_obeyed,
        sensitive_restricted_applicable,
        sensitive_restricted_default_excluded,
        policy_abstention_applicable,
        policy_abstention_correct,
        policy_failure_count,
    }
}

pub(super) fn classify_diagnosis(
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
    missing_event_ids: &[String],
    answer_score: f64,
    abstained: bool,
) -> MemoryBenchDiagnosisOutcome {
    let expected_policy_abstention = task
        .policy
        .as_ref()
        .map(|policy| policy.expected_policy_abstention)
        .unwrap_or(false);
    let missing = !missing_event_ids.is_empty();
    let stored_gold_complete = task.gold_supporting_event_ids.iter().all(|event_id| {
        task.evidence
            .iter()
            .any(|evidence| evidence.event_id == *event_id && evidence.retention_allowed)
    });
    let retrieval_condition = matches!(
        condition,
        MemoryBenchCondition::TruncatedFullContext
            | MemoryBenchCondition::RetrievedMemory
            | MemoryBenchCondition::RememDefault
            | MemoryBenchCondition::Bm25Baseline
            | MemoryBenchCondition::VectorBaseline
            | MemoryBenchCondition::HybridRagBaseline
            | MemoryBenchCondition::SummaryBaseline
    );
    let write_check_condition = matches!(
        condition,
        MemoryBenchCondition::CompleteStoredMemory
            | MemoryBenchCondition::RetrievedMemory
            | MemoryBenchCondition::RememDefault
            | MemoryBenchCondition::Bm25Baseline
            | MemoryBenchCondition::VectorBaseline
            | MemoryBenchCondition::HybridRagBaseline
            | MemoryBenchCondition::SummaryBaseline
    );
    let write_side_gap = missing && write_check_condition && !stored_gold_complete;
    let retrieval_side_gap = missing && retrieval_condition && stored_gold_complete;
    let policy_abstention = expected_policy_abstention && abstained;
    let reader_gap = !missing
        && answer_score < 1.0
        && !policy_abstention
        && condition != MemoryBenchCondition::NoMemory;

    MemoryBenchDiagnosisOutcome {
        write_side_gap,
        retrieval_side_gap,
        reader_gap,
        policy_abstention,
    }
}

pub(super) fn performance_metrics(
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
    reader_input: &str,
    retrieved_count: usize,
) -> MemoryBenchPerformanceMetrics {
    let rows_written = if stores_memory_condition(condition) {
        task.evidence
            .iter()
            .filter(|evidence| evidence.retention_allowed)
            .count() as u64
    } else {
        0
    };
    let ingest_tokens = if rows_written > 0 {
        task.evidence
            .iter()
            .filter(|evidence| evidence.retention_allowed)
            .map(|evidence| estimate_tokens(&evidence.title) + estimate_tokens(&evidence.content))
            .sum()
    } else {
        0
    };
    let query_tokens = estimate_tokens(&task.query) + estimate_tokens(&task.prompt);
    let reader_tokens = estimate_tokens(reader_input);
    let retrieval_latency_ms =
        condition_latency_ms(condition, task.evidence.len(), retrieved_count);
    let end_to_end_latency_ms = retrieval_latency_ms + reader_tokens.saturating_div(8) + 1;
    MemoryBenchPerformanceMetrics {
        ingest_tokens,
        query_tokens,
        reader_tokens,
        retrieval_latency_ms,
        end_to_end_latency_ms,
        rows_written,
    }
}

pub(super) fn failure_decomposition(outcomes: &[MemoryBenchRunOutcome]) -> Value {
    let all = outcomes.iter().collect::<Vec<_>>();
    json!({
        "overall": summarize_failures(&all),
        "by_condition": group_by_condition(outcomes, summarize_failures),
    })
}

pub(super) fn performance_by_condition(outcomes: &[MemoryBenchRunOutcome]) -> Value {
    json!(group_by_condition(outcomes, summarize_performance))
}

pub(super) fn stores_memory_condition(condition: MemoryBenchCondition) -> bool {
    matches!(
        condition,
        MemoryBenchCondition::CompleteStoredMemory
            | MemoryBenchCondition::RetrievedMemory
            | MemoryBenchCondition::RememDefault
            | MemoryBenchCondition::Bm25Baseline
            | MemoryBenchCondition::VectorBaseline
            | MemoryBenchCondition::HybridRagBaseline
            | MemoryBenchCondition::SummaryBaseline
    )
}

fn evidence_retention_allowed(task: &MemoryBenchTask, event_id: &str) -> bool {
    task.evidence
        .iter()
        .find(|evidence| evidence.event_id == event_id)
        .map(|evidence| evidence.retention_allowed)
        .unwrap_or(false)
}

fn group_by_condition<T: Serialize>(
    outcomes: &[MemoryBenchRunOutcome],
    summarize: fn(&[&MemoryBenchRunOutcome]) -> T,
) -> BTreeMap<String, T> {
    let mut grouped: BTreeMap<String, Vec<&MemoryBenchRunOutcome>> = BTreeMap::new();
    for outcome in outcomes {
        grouped
            .entry(outcome.condition.as_str().to_string())
            .or_default()
            .push(outcome);
    }
    grouped
        .into_iter()
        .map(|(condition, runs)| (condition, summarize(&runs)))
        .collect()
}

fn summarize_failures(runs: &[&MemoryBenchRunOutcome]) -> MemoryBenchFailureDecomposition {
    let mut summary = MemoryBenchFailureDecomposition {
        runs: runs.len(),
        ..MemoryBenchFailureDecomposition::default()
    };
    for run in runs {
        if run.diagnosis.write_side_gap {
            summary.write_side_evidence_loss += 1;
        }
        if run.diagnosis.retrieval_side_gap {
            summary.retrieval_miss += 1;
        }
        if run.diagnosis.reader_gap {
            summary.reader_failure += 1;
        }
        if run.diagnosis.policy_abstention {
            summary.policy_abstention += 1;
        }
        if !run.diagnosis.write_side_gap
            && !run.diagnosis.retrieval_side_gap
            && !run.diagnosis.reader_gap
            && !run.diagnosis.policy_abstention
        {
            summary.clean_runs += 1;
        }
    }
    summary
}

fn summarize_performance(runs: &[&MemoryBenchRunOutcome]) -> MemoryBenchPerformanceSummary {
    let mut retrieval_latencies = Vec::new();
    let mut end_to_end_latencies = Vec::new();
    let mut summary = MemoryBenchPerformanceSummary {
        tasks: runs.len(),
        ..MemoryBenchPerformanceSummary::default()
    };
    for run in runs {
        summary.ingest_tokens_mean += run.performance.ingest_tokens as f64;
        summary.query_tokens_mean += run.performance.query_tokens as f64;
        summary.reader_tokens_mean += run.performance.reader_tokens as f64;
        summary.rows_written_mean += run.performance.rows_written as f64;
        retrieval_latencies.push(run.performance.retrieval_latency_ms as f64);
        end_to_end_latencies.push(run.performance.end_to_end_latency_ms as f64);
    }
    if !runs.is_empty() {
        let divisor = runs.len() as f64;
        summary.ingest_tokens_mean /= divisor;
        summary.query_tokens_mean /= divisor;
        summary.reader_tokens_mean /= divisor;
        summary.rows_written_mean /= divisor;
    }
    summary.retrieval_latency_p50_ms = percentile(retrieval_latencies.clone(), 50.0);
    summary.retrieval_latency_p95_ms = percentile(retrieval_latencies, 95.0);
    summary.end_to_end_latency_p50_ms = percentile(end_to_end_latencies.clone(), 50.0);
    summary.end_to_end_latency_p95_ms = percentile(end_to_end_latencies, 95.0);
    summary
}

fn condition_latency_ms(
    condition: MemoryBenchCondition,
    evidence_count: usize,
    retrieved_count: usize,
) -> u64 {
    let base = match condition {
        MemoryBenchCondition::NoMemory => 0,
        MemoryBenchCondition::TruncatedFullContext => 2,
        MemoryBenchCondition::OracleEvidence => 1,
        MemoryBenchCondition::CompleteStoredMemory => 3,
        MemoryBenchCondition::Bm25Baseline => 5,
        MemoryBenchCondition::VectorBaseline => 8,
        MemoryBenchCondition::HybridRagBaseline => 10,
        MemoryBenchCondition::SummaryBaseline => 4,
        MemoryBenchCondition::RetrievedMemory | MemoryBenchCondition::RememDefault => 7,
    };
    base + evidence_count as u64 + retrieved_count as u64
}

fn estimate_tokens(value: &str) -> u64 {
    value
        .split_whitespace()
        .map(|token| token.chars().count().max(1).div_ceil(4) as u64)
        .sum()
}

fn percentile(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let rank = ((percentile / 100.0) * (values.len().saturating_sub(1)) as f64).round() as usize;
    values[rank.min(values.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::memory_bench::types::MemoryBenchEvidence;

    #[test]
    fn write_vs_retrieval_miss_is_not_reader_failure() {
        let task = MemoryBenchTask {
            id: "diagnostic-miss".to_string(),
            category: "diagnostic".to_string(),
            reference_time_epoch: 1,
            prompt: "What is the current API?".to_string(),
            query: "current api".to_string(),
            expected_answer: "Use v2.".to_string(),
            abstention_allowed: false,
            gold_supporting_event_ids: vec!["e1".to_string()],
            forbidden_event_ids: Vec::new(),
            evidence: vec![MemoryBenchEvidence {
                event_id: "e1".to_string(),
                title: "Current API".to_string(),
                content: "Use v2.".to_string(),
                memory_type: "decision".to_string(),
                status: "active".to_string(),
                scope: "project".to_string(),
                topic_key: Some("api".to_string()),
                files: Vec::new(),
                source_anchor: "tracked".to_string(),
                created_at_epoch: Some(1),
                retention_allowed: true,
            }],
            policy: None,
        };
        let diagnosis = classify_diagnosis(
            MemoryBenchCondition::RetrievedMemory,
            &task,
            &["e1".to_string()],
            0.0,
            true,
        );
        assert!(diagnosis.retrieval_side_gap);
        assert!(!diagnosis.write_side_gap);
        assert!(!diagnosis.reader_gap);
        assert!(!diagnosis.policy_abstention);
    }
}
