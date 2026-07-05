use serde::Serialize;

use crate::db;

#[derive(Debug, Clone, Serialize)]
pub(super) struct StatusReport {
    pub version: String,
    pub database: StatusDatabase,
    pub totals: StatusTotals,
    pub embedding: EmbeddingStatus,
    pub raw_archive: RawArchiveStatus,
    pub capture_pipeline: CapturePipelineStatus,
    pub promotion_funnel: PromotionFunnelStatus,
    pub usage_feedback: UsageFeedbackStatus,
    pub pending_observations: PendingObservationStatus,
    pub review_queue: ReviewQueueStatus,
    pub candidate_promotion: Vec<CandidatePromotionStatus>,
    pub jobs: JobStatus,
    pub failure_lifecycle: db::FailureLifecycleStats,
    pub worker_daemon: WorkerDaemonStatus,
    pub latest_session_memory_spend: Option<LatestSessionMemorySpendStatus>,
    pub today: DailyStatus,
    pub top_projects: Vec<TopProjectStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct StatusDatabase {
    pub path: String,
    pub size_bytes: u64,
    pub size_mb: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct StatusTotals {
    pub memories: i64,
    pub observations: i64,
    pub sessions: i64,
    pub raw_messages: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EmbeddingStatus {
    pub configured_provider: String,
    pub fallback_provider: Option<String>,
    pub active_provider: String,
    pub active_model_id: Option<String>,
    pub degraded: bool,
    pub disabled: bool,
    pub unavailable_reason: Option<String>,
    pub degradation_reason: Option<String>,
    pub coverage: EmbeddingCoverageStatus,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EmbeddingCoverageStatus {
    pub embedded: i64,
    pub total: i64,
    pub percent: f64,
    pub mixed_profile_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RawArchiveStatus {
    pub messages: i64,
    pub ingest_failures: i64,
    pub parse_errors: i64,
    pub insert_errors: i64,
    pub latest_failure_epoch: Option<i64>,
    pub latest_failure_age_secs: Option<i64>,
    pub latest_failure_kind: Option<String>,
    pub latest_failure_path: Option<String>,
    pub latest_failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CapturePipelineStatus {
    pub captured: i64,
    pub dropped: i64,
    pub unrecovered_spills: i64,
    pub latest_drop_epoch: Option<i64>,
    pub latest_drop_age_secs: Option<i64>,
    pub latest_drop_reason: Option<String>,
    pub latest_drop_detail: Option<String>,
    pub extract_todo: i64,
    pub extract_running: i64,
    pub extract_expired: i64,
    pub extract_failed: i64,
    pub retryable_replay_ranges: i64,
    pub active_replay_ranges: i64,
    pub quarantined_replay_ranges: i64,
    pub pending_candidates: i64,
    pub pending_graph_candidates: i64,
    pub oldest_task_epoch: Option<i64>,
    pub oldest_task_age_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PromotionFunnelStatus {
    pub captured_events: i64,
    pub observations: i64,
    pub observation_rate_percent: f64,
    pub candidates: i64,
    pub candidate_rate_percent: f64,
    pub promoted: i64,
    pub promoted_rate_percent: f64,
    pub pending_review: i64,
    pub pending_review_rate_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct UsageFeedbackStatus {
    pub citation_events: i64,
    pub citation_line_present_events: i64,
    pub citation_line_present_rate_percent: f64,
    pub matched_events: i64,
    pub match_rate_percent: f64,
    pub inserted_events: i64,
    pub no_citation_events: i64,
    pub unmatched_events: i64,
    pub usage_events: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct PendingObservationStatus {
    pub ready: i64,
    pub delayed: i64,
    pub processing: i64,
    pub expired: i64,
    pub failed: i64,
    pub oldest_ready_epoch: Option<i64>,
    pub oldest_ready_age_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReviewQueueStatus {
    pub pending: i64,
    pub median_age_secs: Option<i64>,
    pub max_age_secs: Option<i64>,
    pub inflow_7d: i64,
    pub resolved_7d: i64,
    pub projects: Vec<ReviewQueueProjectStatus>,
    pub block_reasons: Vec<ReviewQueueBlockReasonStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReviewQueueProjectStatus {
    pub project: Option<String>,
    pub pending: i64,
    pub median_age_secs: Option<i64>,
    pub max_age_secs: Option<i64>,
    pub inflow_7d: i64,
    pub resolved_7d: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReviewQueueBlockReasonStatus {
    pub reason: Option<String>,
    pub pending: i64,
    pub example_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CandidatePromotionStatus {
    pub source_kind: String,
    pub review_status: String,
    pub block_reason: Option<String>,
    pub total: i64,
    pub last_7_days: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct JobStatus {
    pub pending: i64,
    pub processing: i64,
    pub failed: i64,
    pub stuck: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct WorkerDaemonStatus {
    pub health: String,
    pub heartbeat_age_secs: Option<i64>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct LatestSessionMemorySpendStatus {
    pub session_id: String,
    pub project: String,
    pub latest_context_epoch: i64,
    pub context_rows: i64,
    pub context_output_chars: i64,
    pub context_estimated_tokens: i64,
    pub context_emit_count: i64,
    pub context_suppress_count: i64,
    pub ai_usage_attribution: String,
    pub ai_calls: i64,
    pub ai_total_tokens: i64,
    pub ai_estimated_cost_usd: f64,
    pub ai_unattributed_legacy_calls: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DailyStatus {
    pub new_memories: i64,
    pub new_observations: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TopProjectStatus {
    pub project: String,
    pub count: i64,
}
