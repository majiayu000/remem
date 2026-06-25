use super::types::{
    CurrentMemoryContractCaseReport, CurrentMemoryContractMetricSummary,
    CurrentMemoryContractRateMetric, CurrentStateContractMetrics, InjectionAuditContractMetrics,
    StalenessContractMetrics, TemporalContractMetrics, UsageContractMetrics,
};

pub(super) fn summarize_contract_metrics(
    cases: &[CurrentMemoryContractCaseReport],
) -> CurrentMemoryContractMetricSummary {
    CurrentMemoryContractMetricSummary {
        current_state: CurrentStateContractMetrics {
            current: rate(cases, "current_state", &["current"]),
            no_current: rate(cases, "current_state", &["no_current"]),
            unresolved_conflict: rate(cases, "current_state", &["unresolved_conflict"]),
            ambiguous: rate(cases, "current_state", &["ambiguous"]),
        },
        temporal: TemporalContractMetrics {
            invalidated_fact_exclusion: rate(cases, "temporal", &["invalidated_fact_exclusion"]),
            expired_fact_exclusion: rate(cases, "temporal", &["expired_fact_exclusion"]),
            as_of_fact_retrieval: rate(cases, "temporal", &["as_of_fact_retrieval"]),
        },
        staleness: StalenessContractMetrics {
            tracked: rate(cases, "staleness", &["tracked"]),
            untracked: rate(cases, "staleness", &["untracked"]),
            history_tracked: rate(cases, "staleness", &["history_tracked"]),
            verify_before_trust: rate(cases, "staleness", &["verify_before_trust"]),
            error: rate(cases, "staleness", &["error"]),
        },
        injection: InjectionAuditContractMetrics {
            audit_injected: rate(cases, "injection", &["audit_injected"]),
            audit_dropped: rate(cases, "injection", &["audit_dropped"]),
            audit_abstained: rate(cases, "injection", &["audit_abstained"]),
            output_gate_recorded: rate(cases, "injection", &["output_gate_recorded"]),
        },
        usage: UsageContractMetrics {
            citation_event_matched: rate(cases, "usage", &["citation_event_matched"]),
            citation_event_no_citation: rate(cases, "usage", &["citation_event_no_citation"]),
            usage_event_linked_to_injection_item: rate(
                cases,
                "usage",
                &["usage_event_linked_to_injection_item"],
            ),
        },
        all_checks_passed: cases.iter().all(|case| case.pass),
    }
}

fn rate(
    cases: &[CurrentMemoryContractCaseReport],
    category: &str,
    ids: &[&str],
) -> CurrentMemoryContractRateMetric {
    let passed = ids
        .iter()
        .filter(|id| {
            cases
                .iter()
                .any(|case| case.category == category && case.id == **id && case.pass)
        })
        .count();
    CurrentMemoryContractRateMetric::new(passed, ids.len())
}

pub(super) fn push_case(
    cases: &mut Vec<CurrentMemoryContractCaseReport>,
    category: &str,
    id: &str,
    expected: impl Into<String>,
    actual: impl Into<String>,
    pass: bool,
) {
    cases.push(CurrentMemoryContractCaseReport {
        id: id.to_string(),
        category: category.to_string(),
        expected: expected.into(),
        actual: actual.into(),
        pass,
    });
}
