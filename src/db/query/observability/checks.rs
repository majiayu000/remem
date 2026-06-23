use super::types::{
    CaptureObservabilityMetrics, ContextInjectionObservabilityMetrics, ObservabilityCheck,
    PromotionObservabilityMetrics, QueueObservabilityMetrics, StalenessObservabilityMetrics,
    TemporalFactObservabilityMetrics, UsageFeedbackObservabilityMetrics,
    WorkerObservabilityMetrics,
};

pub(super) fn observability_checks(
    capture: &CaptureObservabilityMetrics,
    promotion: &PromotionObservabilityMetrics,
    context: &ContextInjectionObservabilityMetrics,
    usage: &UsageFeedbackObservabilityMetrics,
    facts: &TemporalFactObservabilityMetrics,
    staleness: &StalenessObservabilityMetrics,
    queue: &QueueObservabilityMetrics,
    worker: &WorkerObservabilityMetrics,
) -> Vec<ObservabilityCheck> {
    let mut checks = Vec::new();
    push_capture_checks(&mut checks, capture);
    push_promotion_checks(&mut checks, promotion);
    push_context_checks(&mut checks, context);
    push_usage_checks(&mut checks, usage);
    push_fact_checks(&mut checks, facts);
    push_staleness_checks(&mut checks, staleness);
    push_queue_checks(&mut checks, queue, capture);
    push_worker_checks(&mut checks, worker);
    checks
}

fn push_capture_checks(
    checks: &mut Vec<ObservabilityCheck>,
    metrics: &CaptureObservabilityMetrics,
) {
    if metrics.actionable_capture_drops > 0 || metrics.unrecovered_capture_spills > 0 {
        checks.push(
            ObservabilityCheck::new(
                "capture_drop_actionable",
                "warn",
                "capture",
                "capture drops require recovery or spill inspection",
            )
            .metric("actionable_capture_drops", metrics.actionable_capture_drops)
            .metric(
                "unrecovered_capture_spills",
                metrics.unrecovered_capture_spills,
            )
            .action("run `remem status --json`")
            .action("inspect capture-drop spill files before retrying ingestion"),
        );
    }
}

fn push_promotion_checks(
    checks: &mut Vec<ObservabilityCheck>,
    metrics: &PromotionObservabilityMetrics,
) {
    if metrics.observations > 0 && metrics.candidates == 0 {
        checks.push(
            ObservabilityCheck::new(
                "promotion_funnel_no_candidates",
                "warn",
                "promotion",
                "observations exist but no memory candidates were produced",
            )
            .metric("observations", metrics.observations)
            .metric("candidates", metrics.candidates)
            .action("run `remem worker --once`")
            .action("inspect extraction and promotion logs"),
        );
    } else if metrics.candidates > 0
        && metrics.promoted == 0
        && metrics.pending_review == metrics.candidates
    {
        checks.push(
            ObservabilityCheck::new(
                "promotion_funnel_all_pending_review",
                "info",
                "promotion",
                "all memory candidates are still pending review",
            )
            .metric("candidates", metrics.candidates)
            .metric("pending_review", metrics.pending_review)
            .action("review memory candidates before expecting active memories"),
        );
    }
}

fn push_context_checks(
    checks: &mut Vec<ObservabilityCheck>,
    metrics: &ContextInjectionObservabilityMetrics,
) {
    if !metrics.output_table_exists || !metrics.item_table_exists {
        checks.push(
            ObservabilityCheck::new(
                "context_injection_audit_missing",
                "warn",
                "context",
                "context injection audit tables are missing",
            )
            .metric(
                "output_table_exists",
                metric_bool(metrics.output_table_exists),
            )
            .metric("item_table_exists", metric_bool(metrics.item_table_exists))
            .action("run database migrations before reading context observability"),
        );
    }
}

fn push_usage_checks(
    checks: &mut Vec<ObservabilityCheck>,
    metrics: &UsageFeedbackObservabilityMetrics,
) {
    if !metrics.citation_table_exists || !metrics.usage_table_exists {
        checks.push(
            ObservabilityCheck::new(
                "memory_usage_feedback_missing",
                "warn",
                "usage",
                "memory citation or usage feedback tables are missing",
            )
            .metric(
                "citation_table_exists",
                metric_bool(metrics.citation_table_exists),
            )
            .metric(
                "usage_table_exists",
                metric_bool(metrics.usage_table_exists),
            )
            .action("run database migrations before reading usage feedback"),
        );
    } else if metrics.citation_events > 0 && metrics.matched_events == 0 {
        checks.push(
            ObservabilityCheck::new(
                "memory_usage_feedback_no_matches",
                "warn",
                "usage",
                "citation events exist but none matched injected memories",
            )
            .metric("citation_events", metrics.citation_events)
            .metric("matched_events", metrics.matched_events)
            .action("verify injected citation ids use the `memory:#<id>` contract"),
        );
    }
}

fn push_fact_checks(
    checks: &mut Vec<ObservabilityCheck>,
    metrics: &TemporalFactObservabilityMetrics,
) {
    if !metrics.table_exists {
        checks.push(
            ObservabilityCheck::new(
                "temporal_facts_missing",
                "warn",
                "facts",
                "memory_facts table is missing",
            )
            .action("run database migrations before reading temporal fact observability"),
        );
    } else if metrics.total_rows > 0 && metrics.retrieval_eligible_rows == 0 {
        checks.push(
            ObservabilityCheck::new(
                "temporal_facts_no_retrieval_eligible",
                "warn",
                "facts",
                "temporal facts exist but none are retrieval eligible",
            )
            .metric("total_rows", metrics.total_rows)
            .metric("retrieval_eligible_rows", metrics.retrieval_eligible_rows)
            .action("inspect fact invalidation, expiry, and source-memory links"),
        );
    }
}

fn push_staleness_checks(
    checks: &mut Vec<ObservabilityCheck>,
    metrics: &StalenessObservabilityMetrics,
) {
    if !metrics.memory_table_exists {
        checks.push(
            ObservabilityCheck::new(
                "staleness_memory_table_missing",
                "warn",
                "staleness",
                "memories table is missing",
            )
            .action("run database migrations before reading staleness observability"),
        );
    } else if metrics.error_count > 0 {
        checks.push(
            ObservabilityCheck::new(
                "staleness_source_anchor_error",
                "warn",
                "staleness",
                "one or more memories produced source-anchor staleness errors",
            )
            .metric("error_count", metrics.error_count)
            .action("inspect memory evidence files and source-anchor metadata"),
        );
    }
}

fn push_queue_checks(
    checks: &mut Vec<ObservabilityCheck>,
    queue: &QueueObservabilityMetrics,
    capture: &CaptureObservabilityMetrics,
) {
    let queue_failures = queue.failed_pending_observations
        + queue.expired_processing_pending_observations
        + queue.failed_jobs
        + queue.stuck_jobs
        + capture.failed_extraction_tasks
        + capture.expired_processing_extraction_tasks;
    if queue_failures > 0 || queue.retryable_extraction_replay_ranges > 0 {
        checks.push(
            ObservabilityCheck::new(
                "pending_queue_recovery_needed",
                "warn",
                "queue",
                "pending, extraction, or job queues have recoverable failures",
            )
            .metric("queue_failures", queue_failures)
            .metric(
                "retryable_extraction_replay_ranges",
                queue.retryable_extraction_replay_ranges,
            )
            .action("run `remem status --json`")
            .action("run `remem worker --once`"),
        );
    }
}

fn push_worker_checks(checks: &mut Vec<ObservabilityCheck>, metrics: &WorkerObservabilityMetrics) {
    if !metrics.daemon_healthy {
        checks.push(
            ObservabilityCheck::new(
                "worker_daemon_not_healthy",
                "info",
                "worker",
                "daemon worker heartbeat is missing or stale",
            )
            .metric(
                "heartbeat_age_secs",
                metrics.heartbeat_age_secs.unwrap_or_default(),
            )
            .action("when Stop hooks are installed, they run `remem worker --once`"),
        );
    }
}

fn metric_bool(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}
