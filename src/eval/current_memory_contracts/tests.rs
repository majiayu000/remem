use super::run_current_memory_contracts_eval;
use super::types::CurrentMemoryContractRateMetric;

#[test]
fn builtin_current_memory_contracts_score_all_acceptance_points() {
    let report = run_current_memory_contracts_eval().unwrap();

    assert!(report.metrics.all_checks_passed);
    assert!(report.failing_examples.is_empty());
    assert_eq!(report.cases.len(), 17);
    assert_eq!(
        report.metrics.current_state.current,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.current_state.no_current,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.current_state.unresolved_conflict,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.current_state.ambiguous,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.temporal.invalidated_fact_exclusion,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.temporal.expired_fact_exclusion,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.temporal.as_of_fact_retrieval,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.staleness.tracked,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.staleness.untracked,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.staleness.history_tracked,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.staleness.verify_before_trust,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.staleness.error,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.injection.audit_injected,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.injection.audit_dropped,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.injection.audit_abstained,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.usage.citation_event_matched,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
    assert_eq!(
        report.metrics.usage.usage_event_linked_to_injection_item,
        CurrentMemoryContractRateMetric::new(1, 1)
    );
}
