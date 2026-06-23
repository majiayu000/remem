use std::collections::BTreeMap;

use serde::Serialize;

pub const OBSERVABILITY_SCHEMA_VERSION: u32 = 1;
pub const CURRENT_MEMORY_CONTRACT_SPEC_PATH: &str = "docs/specs/current-memory-contracts/TECH.md";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObservabilityReport {
    pub schema_version: u32,
    pub generated_at_epoch: i64,
    pub spec_path: &'static str,
    pub checks: Vec<ObservabilityCheck>,
    pub metrics: ObservabilityMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservabilityCheck {
    pub code: String,
    pub severity: &'static str,
    pub scope: &'static str,
    pub message: String,
    pub metrics: BTreeMap<String, i64>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ObservabilityMetrics {
    pub capture: CaptureObservabilityMetrics,
    pub promotion: PromotionObservabilityMetrics,
    pub context_injection: ContextInjectionObservabilityMetrics,
    pub usage_feedback: UsageFeedbackObservabilityMetrics,
    pub temporal_facts: TemporalFactObservabilityMetrics,
    pub staleness: StalenessObservabilityMetrics,
    pub queue: QueueObservabilityMetrics,
    pub worker: WorkerObservabilityMetrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CaptureObservabilityMetrics {
    pub captured_events: i64,
    pub capture_drop_events: i64,
    pub actionable_capture_drops: i64,
    pub unrecovered_capture_spills: i64,
    pub pending_extraction_tasks: i64,
    pub processing_extraction_tasks: i64,
    pub expired_processing_extraction_tasks: i64,
    pub failed_extraction_tasks: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PromotionObservabilityMetrics {
    pub observations: i64,
    pub candidates: i64,
    pub promoted: i64,
    pub pending_review: i64,
    pub candidate_rate_percent: f64,
    pub promoted_rate_percent: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ContextInjectionObservabilityMetrics {
    pub output_table_exists: bool,
    pub item_table_exists: bool,
    pub output_rows: i64,
    pub output_emit_count: i64,
    pub output_suppress_count: i64,
    pub output_modes: Vec<CountBucket>,
    pub item_rows: i64,
    pub item_statuses: Vec<CountBucket>,
    pub item_channels: Vec<CountBucket>,
    pub item_drop_reasons: Vec<CountBucket>,
    pub item_staleness_source_anchors: Vec<CountBucket>,
    pub item_staleness_ages: Vec<CountBucket>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageFeedbackObservabilityMetrics {
    pub citation_table_exists: bool,
    pub usage_table_exists: bool,
    pub citation_events: i64,
    pub citation_line_present_events: i64,
    pub matched_events: i64,
    pub inserted_events: i64,
    pub no_citation_events: i64,
    pub unmatched_events: i64,
    pub usage_events: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TemporalFactObservabilityMetrics {
    pub table_exists: bool,
    pub total_rows: i64,
    pub retrieval_eligible_rows: i64,
    pub invalidated_rows: i64,
    pub expired_rows: i64,
    pub orphan_source_memory_rows: i64,
    pub unlinked_source_rows: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StalenessObservabilityMetrics {
    pub memory_table_exists: bool,
    pub total_memories: i64,
    pub source_anchors: Vec<CountBucket>,
    pub ages: Vec<CountBucket>,
    pub error_count: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QueueObservabilityMetrics {
    pub pending_observations: i64,
    pub ready_pending_observations: i64,
    pub delayed_pending_observations: i64,
    pub processing_pending_observations: i64,
    pub expired_processing_pending_observations: i64,
    pub failed_pending_observations: i64,
    pub pending_jobs: i64,
    pub processing_jobs: i64,
    pub failed_jobs: i64,
    pub stuck_jobs: i64,
    pub retryable_extraction_replay_ranges: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkerObservabilityMetrics {
    pub daemon_healthy: bool,
    pub heartbeat_age_secs: Option<i64>,
    pub heartbeat_owner_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CountBucket {
    pub value: String,
    pub count: i64,
}

impl ObservabilityReport {
    pub fn unavailable(generated_at_epoch: i64, reason: impl Into<String>) -> Self {
        Self {
            schema_version: OBSERVABILITY_SCHEMA_VERSION,
            generated_at_epoch,
            spec_path: CURRENT_MEMORY_CONTRACT_SPEC_PATH,
            checks: vec![ObservabilityCheck::new(
                "observability_database_unavailable",
                "warn",
                "database",
                reason,
            )
            .action("open the remem database before requesting runtime observability")],
            metrics: ObservabilityMetrics::default(),
        }
    }
}

impl ObservabilityCheck {
    pub fn new(
        code: impl Into<String>,
        severity: &'static str,
        scope: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            scope,
            message: message.into(),
            metrics: BTreeMap::new(),
            actions: Vec::new(),
        }
    }

    pub fn metric(mut self, key: &'static str, value: i64) -> Self {
        self.metrics.insert(key.to_string(), value);
        self
    }

    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.actions.push(action.into());
        self
    }
}
