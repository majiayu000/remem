use crate::eval::golden::MetricAverages;

use super::{
    DefaultDecision, DefaultDecisionKind, DefaultFlipCriteria, ProviderComparisonRow,
    COLD_START_EMBEDDING_LATENCY_BUDGET_MS, EPSILON, EXISTING_REGRESSION_BUDGET,
    QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS,
};

pub(super) fn build_default_decision(providers: &[ProviderComparisonRow]) -> DefaultDecision {
    let feature_hash = provider_row(providers, "feature-hash");
    let local = provider_row(providers, "local");
    let api = provider_row(providers, "api");
    let local_available = local.is_some_and(|row| row.available);
    let api_reference_available = api.is_some_and(|row| row.available);
    let provider_comparison_slice_present = local
        .and_then(|row| row.provider_comparison_slice.as_ref())
        .is_some_and(|slice| slice.scored_queries > 0);
    let provider_comparison_slice_improves = feature_hash
        .zip(local)
        .is_some_and(|(baseline, local)| provider_slice_improves(baseline, local));
    let paraphrase_slice_improves = feature_hash
        .zip(local)
        .is_some_and(|(baseline, local)| existing_slice_improves(baseline, local, "paraphrase"));
    let existing_slices_within_budget = feature_hash
        .zip(local)
        .is_some_and(|(baseline, local)| existing_slices_within_budget(baseline, local));
    let cold_start_embedding_latency_within_budget = local.is_some_and(|row| {
        row.cold_start_embedding_latency_ms
            .is_some_and(|latency| latency <= COLD_START_EMBEDDING_LATENCY_BUDGET_MS)
    });
    let query_embedding_latency_within_budget = local.is_some_and(|row| {
        row.query_embedding_latency_p95_ms
            .is_some_and(|latency| latency <= QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS)
    });

    let criteria = DefaultFlipCriteria {
        local_available,
        api_reference_available,
        provider_comparison_slice_present,
        provider_comparison_slice_improves,
        paraphrase_slice_improves,
        existing_slices_within_budget,
        cold_start_embedding_latency_within_budget,
        query_embedding_latency_within_budget,
    };
    let mut blockers = Vec::new();
    if !criteria.local_available {
        blockers.push(provider_blocker(local, "local provider unavailable"));
    }
    if !criteria.api_reference_available {
        blockers.push(provider_blocker(api, "api reference unavailable"));
    }
    if !criteria.provider_comparison_slice_present {
        blockers.push("provider_comparison slice has no scored local queries".to_string());
    }
    if !criteria.provider_comparison_slice_improves {
        blockers.push(
            "local provider did not improve provider_comparison evidence recall over feature-hash"
                .to_string(),
        );
    }
    if !criteria.paraphrase_slice_improves {
        blockers.push(
            "local provider did not improve paraphrase evidence recall over feature-hash"
                .to_string(),
        );
    }
    if !criteria.existing_slices_within_budget {
        blockers.push("local provider regressed existing golden slices beyond budget".to_string());
    }
    if !criteria.cold_start_embedding_latency_within_budget {
        blockers.push(format!(
            "local cold-start embedding latency exceeded {:.0}ms budget or was not measured",
            COLD_START_EMBEDDING_LATENCY_BUDGET_MS
        ));
    }
    if !criteria.query_embedding_latency_within_budget {
        blockers.push(format!(
            "local warm-query embedding p95 exceeded {:.0}ms budget or was not measured",
            QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS
        ));
    }
    let change_default = blockers.is_empty();
    let decision = if change_default {
        DefaultDecisionKind::FlipToLocal
    } else {
        DefaultDecisionKind::KeepFeatureHash
    };
    let decision_reason = if change_default {
        "Local semantic embeddings improved both paraphrase and provider-comparison evidence recall while satisfying the remaining regression, cold-start, warm-query latency, and API-reference criteria.".to_string()
    } else {
        format!(
            "Keep the default provider unchanged until GH-716 blockers are cleared: {}",
            blockers.join("; ")
        )
    };

    DefaultDecision {
        change_default,
        decision,
        decision_reason,
        criteria,
        blockers,
    }
}

pub(super) fn provider_row<'a>(
    providers: &'a [ProviderComparisonRow],
    provider: &str,
) -> Option<&'a ProviderComparisonRow> {
    providers.iter().find(|row| row.provider == provider)
}

fn provider_blocker(row: Option<&ProviderComparisonRow>, fallback: &str) -> String {
    row.and_then(|row| row.unavailable_reason.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn provider_slice_improves(
    feature_hash: &ProviderComparisonRow,
    local: &ProviderComparisonRow,
) -> bool {
    metric_delta(
        feature_hash
            .provider_comparison_slice
            .as_ref()
            .and_then(|slice| slice.metrics.as_ref()),
        local
            .provider_comparison_slice
            .as_ref()
            .and_then(|slice| slice.metrics.as_ref()),
        |metrics| metrics.evidence_recall_at_k,
    )
    .is_some_and(|delta| delta > EPSILON)
}

fn existing_slice_improves(
    feature_hash: &ProviderComparisonRow,
    local: &ProviderComparisonRow,
    slice: &str,
) -> bool {
    metric_delta(
        feature_hash
            .existing_slice_details
            .get(slice)
            .and_then(|category| category.metrics.as_ref()),
        local
            .existing_slice_details
            .get(slice)
            .and_then(|category| category.metrics.as_ref()),
        |metrics| metrics.evidence_recall_at_k,
    )
    .is_some_and(|delta| delta > EPSILON)
}

pub(super) fn existing_slices_within_budget(
    feature_hash: &ProviderComparisonRow,
    local: &ProviderComparisonRow,
) -> bool {
    if feature_hash.existing_slice_details.is_empty() {
        return false;
    }
    feature_hash
        .existing_slice_details
        .iter()
        .all(|(slice, baseline)| {
            local
                .existing_slice_details
                .get(slice)
                .is_some_and(|candidate| {
                    category_within_budget(baseline, candidate, EXISTING_REGRESSION_BUDGET)
                })
        })
}

fn category_within_budget(
    baseline: &crate::eval::golden::CategoryEvaluation,
    candidate: &crate::eval::golden::CategoryEvaluation,
    budget: f64,
) -> bool {
    if baseline.total_queries != candidate.total_queries
        || baseline.scored_queries != candidate.scored_queries
        || baseline.abstention_queries != candidate.abstention_queries
    {
        return false;
    }

    let mut checked = false;
    if baseline.scored_queries > 0 {
        checked = true;
        let Some((baseline_metrics, candidate_metrics)) =
            baseline.metrics.as_ref().zip(candidate.metrics.as_ref())
        else {
            return false;
        };
        if !metrics_within_budget(baseline_metrics, candidate_metrics, budget) {
            return false;
        }
    }
    if baseline.abstention_queries > 0 {
        checked = true;
        let baseline_rate = baseline.abstention_passed as f64 / baseline.abstention_queries as f64;
        let candidate_rate =
            candidate.abstention_passed as f64 / candidate.abstention_queries as f64;
        if candidate_rate + budget + EPSILON < baseline_rate {
            return false;
        }
    }
    checked
}

fn metric_delta(
    baseline: Option<&MetricAverages>,
    candidate: Option<&MetricAverages>,
    value: impl Fn(&MetricAverages) -> f64,
) -> Option<f64> {
    Some(value(candidate?) - value(baseline?))
}

fn metrics_within_budget(
    baseline: &MetricAverages,
    candidate: &MetricAverages,
    budget: f64,
) -> bool {
    candidate.hit_at_k + budget + EPSILON >= baseline.hit_at_k
        && candidate.mrr_at_10 + budget + EPSILON >= baseline.mrr_at_10
        && candidate.precision_at_k + budget + EPSILON >= baseline.precision_at_k
        && candidate.recall_at_k + budget + EPSILON >= baseline.recall_at_k
        && candidate.ndcg_at_10 + budget + EPSILON >= baseline.ndcg_at_10
        && candidate.evidence_recall_at_k + budget + EPSILON >= baseline.evidence_recall_at_k
}
