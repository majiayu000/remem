use serde::Serialize;

pub type CurrentMemoryContractRateMetric = crate::eval::governance::RateMetric;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CurrentMemoryContractEvalReport {
    pub metadata: CurrentMemoryContractEvalMetadata,
    pub metrics: CurrentMemoryContractMetricSummary,
    pub cases: Vec<CurrentMemoryContractCaseReport>,
    pub failing_examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CurrentMemoryContractEvalMetadata {
    pub corpus: String,
    pub storage: String,
    pub real_db_touched: bool,
    pub project: String,
    pub host: String,
    pub scenarios: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CurrentMemoryContractMetricSummary {
    pub current_state: CurrentStateContractMetrics,
    pub temporal: TemporalContractMetrics,
    pub staleness: StalenessContractMetrics,
    pub injection: InjectionAuditContractMetrics,
    pub usage: UsageContractMetrics,
    pub all_checks_passed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CurrentStateContractMetrics {
    pub current: CurrentMemoryContractRateMetric,
    pub no_current: CurrentMemoryContractRateMetric,
    pub unresolved_conflict: CurrentMemoryContractRateMetric,
    pub ambiguous: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TemporalContractMetrics {
    pub invalidated_fact_exclusion: CurrentMemoryContractRateMetric,
    pub expired_fact_exclusion: CurrentMemoryContractRateMetric,
    pub as_of_fact_retrieval: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StalenessContractMetrics {
    pub tracked: CurrentMemoryContractRateMetric,
    pub untracked: CurrentMemoryContractRateMetric,
    pub verify_before_trust: CurrentMemoryContractRateMetric,
    pub error: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InjectionAuditContractMetrics {
    pub audit_injected: CurrentMemoryContractRateMetric,
    pub audit_dropped: CurrentMemoryContractRateMetric,
    pub audit_abstained: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UsageContractMetrics {
    pub citation_event_matched: CurrentMemoryContractRateMetric,
    pub usage_event_linked_to_injection_item: CurrentMemoryContractRateMetric,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CurrentMemoryContractCaseReport {
    pub id: String,
    pub category: String,
    pub expected: String,
    pub actual: String,
    pub pass: bool,
}
