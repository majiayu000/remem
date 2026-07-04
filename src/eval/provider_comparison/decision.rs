use crate::eval::golden::MetricAverages;

use super::{
    DefaultDecision, DefaultDecisionKind, DefaultFlipCriteria, ProviderComparisonRow, EPSILON,
    EXISTING_REGRESSION_BUDGET, QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS,
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
    let existing_slices_within_budget = feature_hash
        .zip(local)
        .is_some_and(|(baseline, local)| existing_slices_within_budget(baseline, local));
    let query_embedding_latency_within_budget = local.is_some_and(|row| {
        row.query_embedding_latency_p95_ms
            .is_some_and(|latency| latency <= QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS)
    });

    let criteria = DefaultFlipCriteria {
        local_available,
        api_reference_available,
        provider_comparison_slice_present,
        provider_comparison_slice_improves,
        existing_slices_within_budget,
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
    if !criteria.existing_slices_within_budget {
        blockers.push("local provider regressed existing golden slices beyond budget".to_string());
    }
    if !criteria.query_embedding_latency_within_budget {
        blockers.push(format!(
            "local query embedding p95 exceeded {:.0}ms budget or was not measured",
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
        "Local semantic embeddings satisfied the provider-comparison quality, regression, latency, and API-reference criteria.".to_string()
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

pub(super) fn existing_slices_within_budget(
    feature_hash: &ProviderComparisonRow,
    local: &ProviderComparisonRow,
) -> bool {
    if feature_hash.existing_slice_details.is_empty() {
        return false;
    }
    let mut checked_slices = 0usize;
    let all_checked_slices_pass = feature_hash
        .existing_slice_details
        .iter()
        .filter_map(|(slice, baseline)| {
            let baseline = baseline.metrics.as_ref()?;
            let candidate = local.existing_slice_details.get(slice)?.metrics.as_ref()?;
            Some((baseline, candidate))
        })
        .all(|(baseline, candidate)| {
            checked_slices += 1;
            metrics_within_budget(baseline, candidate, EXISTING_REGRESSION_BUDGET)
        });

    checked_slices > 0 && all_checked_slices_pass
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
