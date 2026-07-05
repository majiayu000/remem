use rusqlite::Connection;

use super::types::{Check, Status};
use crate::memory_candidate::review_stats::{query_review_queue_stats, ReviewQueueStats};

const CHECK_NAME: &str = "Review queue";

/// Block reasons that a candidate can never outgrow: the gate predicate reads
/// a field that is fixed at extraction time, so only human review can resolve
/// the row.
const STRUCTURAL_BLOCK_REASONS: &[&str] = &[
    "risk_class_not_low",
    "memory_type_not_auto_promotable",
    "summary_type_not_allowlisted",
    "summary_risk_above_medium",
    "scope_not_project",
    "contains_unsafe_marker",
    "missing_evidence_ids",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ReviewQueueThresholds {
    pub median_age_warn_days: i64,
    pub inflow_ratio_warn: i64,
    pub backlog_min: i64,
    pub block_share_warn_percent: i64,
}

impl Default for ReviewQueueThresholds {
    fn default() -> Self {
        Self {
            median_age_warn_days: 14,
            inflow_ratio_warn: 3,
            backlog_min: 50,
            block_share_warn_percent: 60,
        }
    }
}

impl ReviewQueueThresholds {
    /// Env overrides until #588 decides a config home.
    fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            median_age_warn_days: env_i64(
                "REMEM_REVIEW_MEDIAN_AGE_WARN_DAYS",
                defaults.median_age_warn_days,
            ),
            inflow_ratio_warn: env_i64(
                "REMEM_REVIEW_INFLOW_RATIO_WARN",
                defaults.inflow_ratio_warn,
            ),
            backlog_min: env_i64("REMEM_REVIEW_BACKLOG_MIN", defaults.backlog_min),
            block_share_warn_percent: env_i64(
                "REMEM_REVIEW_BLOCK_SHARE_WARN_PERCENT",
                defaults.block_share_warn_percent,
            ),
        }
    }
}

fn env_i64(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

pub(super) fn check_review_queue(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new(CHECK_NAME, Status::Warn, "cannot open database");
    };
    let now = chrono::Utc::now().timestamp();
    let stats = match query_review_queue_stats(conn, now) {
        Ok(stats) => stats,
        Err(err) => {
            return Check::new(
                CHECK_NAME,
                Status::Warn,
                format!("cannot load review queue stats: {}", err),
            );
        }
    };
    evaluate_review_queue(&stats, ReviewQueueThresholds::from_env())
}

pub(super) fn evaluate_review_queue(
    stats: &ReviewQueueStats,
    thresholds: ReviewQueueThresholds,
) -> Check {
    let median_age_days = stats.pending_median_age_secs.unwrap_or(0) / 86_400;
    let mut detail = format!(
        "pending={}, median_age_days={}, inflow_7d={}, resolved_7d={} \
         (thresholds: median_age>{}d, inflow>{}x resolved with backlog>{})",
        stats.pending_total,
        median_age_days,
        stats.inflow_7d,
        stats.resolved_7d,
        thresholds.median_age_warn_days,
        thresholds.inflow_ratio_warn,
        thresholds.backlog_min,
    );

    let mut warnings = Vec::new();
    if stats.pending_total > 0 && median_age_days > thresholds.median_age_warn_days {
        warnings.push(format!(
            "median queue age {}d exceeds {}d; review `remem review list` or batch operations",
            median_age_days, thresholds.median_age_warn_days
        ));
    }
    if stats.pending_total > thresholds.backlog_min
        && stats.inflow_7d > stats.resolved_7d.max(1) * thresholds.inflow_ratio_warn
    {
        warnings.push(format!(
            "7-day inflow {} outpaces resolved {} by more than {}x with backlog {}",
            stats.inflow_7d, stats.resolved_7d, thresholds.inflow_ratio_warn, stats.pending_total
        ));
    }
    if let Some(deadlock) = dominant_structural_block(stats, thresholds.block_share_warn_percent) {
        warnings.push(deadlock);
    }

    if warnings.is_empty() {
        Check::new(CHECK_NAME, Status::Ok, detail)
    } else {
        detail.push_str("; ");
        detail.push_str(&warnings.join("; "));
        Check::new(CHECK_NAME, Status::Warn, detail)
    }
}

pub(super) fn dominant_structural_block(
    stats: &ReviewQueueStats,
    share_percent: i64,
) -> Option<String> {
    if stats.pending_total == 0 {
        return None;
    }
    for reason in &stats.block_reasons {
        let Some(name) = reason.reason.as_deref() else {
            continue;
        };
        if !STRUCTURAL_BLOCK_REASONS.contains(&name) {
            continue;
        }
        if (reason.pending as i128) * 100 > (share_percent as i128) * (stats.pending_total as i128)
        {
            let share = (reason.pending as f64 * 100.0) / stats.pending_total as f64;
            return Some(format!(
                "gate deadlock: block reason '{}' holds {:.1}% of pending ({} rows) and is \
                 gate-ineligible by construction; inspect `remem review blocked`, then \
                 approve-batch or discard-batch",
                name, share, reason.pending
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_candidate::review_stats::ReviewQueueBlockReason;

    fn stats(
        pending: i64,
        median_age_secs: Option<i64>,
        inflow_7d: i64,
        resolved_7d: i64,
        block_reasons: Vec<ReviewQueueBlockReason>,
    ) -> ReviewQueueStats {
        ReviewQueueStats {
            pending_total: pending,
            pending_median_age_secs: median_age_secs,
            pending_max_age_secs: median_age_secs,
            inflow_7d,
            resolved_7d,
            projects: Vec::new(),
            block_reasons,
        }
    }

    #[test]
    fn median_age_just_below_threshold_is_ok() {
        let check = evaluate_review_queue(
            &stats(10, Some(14 * 86_400), 0, 0, Vec::new()),
            ReviewQueueThresholds::default(),
        );
        assert_eq!(check.status, Status::Ok);
    }

    #[test]
    fn median_age_just_above_threshold_warns() {
        let check = evaluate_review_queue(
            &stats(10, Some(15 * 86_400), 0, 0, Vec::new()),
            ReviewQueueThresholds::default(),
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("median queue age"));
    }

    #[test]
    fn inflow_ratio_needs_backlog_above_minimum() {
        let thresholds = ReviewQueueThresholds::default();
        // Backlog exactly at the minimum: no warning even with high inflow.
        let at_min = evaluate_review_queue(&stats(50, Some(0), 40, 1, Vec::new()), thresholds);
        // Backlog above the minimum with inflow > 3x resolved: warns.
        let above = evaluate_review_queue(&stats(51, Some(0), 40, 1, Vec::new()), thresholds);
        // Inflow exactly at 3x resolved: no warning.
        let ratio_at = evaluate_review_queue(&stats(51, Some(0), 3, 1, Vec::new()), thresholds);

        assert_eq!(at_min.status, Status::Ok);
        assert_eq!(above.status, Status::Warn);
        assert!(above.detail.contains("outpaces resolved"));
        assert_eq!(ratio_at.status, Status::Ok);
    }

    #[test]
    fn structural_block_reason_above_share_is_a_deadlock() {
        let reasons = vec![ReviewQueueBlockReason {
            reason: Some("risk_class_not_low".to_string()),
            pending: 61,
            example_ids: vec![1, 2, 3],
        }];
        let check = evaluate_review_queue(
            &stats(100, Some(0), 0, 0, reasons),
            ReviewQueueThresholds::default(),
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("gate deadlock"));
    }

    #[test]
    fn summary_structural_block_reason_above_share_is_a_deadlock() {
        let reasons = vec![ReviewQueueBlockReason {
            reason: Some("summary_type_not_allowlisted".to_string()),
            pending: 61,
            example_ids: vec![1, 2, 3],
        }];
        let check = evaluate_review_queue(
            &stats(100, Some(0), 0, 0, reasons),
            ReviewQueueThresholds::default(),
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("summary_type_not_allowlisted"));
    }

    #[test]
    fn missing_evidence_block_reason_above_share_is_a_deadlock() {
        let reasons = vec![ReviewQueueBlockReason {
            reason: Some("missing_evidence_ids".to_string()),
            pending: 61,
            example_ids: vec![1, 2, 3],
        }];
        let check = evaluate_review_queue(
            &stats(100, Some(0), 0, 0, reasons),
            ReviewQueueThresholds::default(),
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("missing_evidence_ids"));
    }

    #[test]
    fn structural_block_reason_fraction_above_share_is_a_deadlock() {
        let reasons = vec![ReviewQueueBlockReason {
            reason: Some("risk_class_not_low".to_string()),
            pending: 61,
            example_ids: vec![1, 2, 3],
        }];
        let check = evaluate_review_queue(
            &stats(101, Some(0), 0, 0, reasons),
            ReviewQueueThresholds::default(),
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("60.4%"));
    }

    #[test]
    fn non_structural_block_reason_never_reports_deadlock() {
        let reasons = vec![ReviewQueueBlockReason {
            reason: Some("confidence_below_threshold".to_string()),
            pending: 100,
            example_ids: vec![1],
        }];
        assert_eq!(
            dominant_structural_block(&stats(100, None, 0, 0, reasons), 60),
            None
        );
    }

    #[test]
    fn structural_block_reason_at_share_boundary_is_ok() {
        let reasons = vec![ReviewQueueBlockReason {
            reason: Some("risk_class_not_low".to_string()),
            pending: 60,
            example_ids: vec![1],
        }];
        assert_eq!(
            dominant_structural_block(&stats(100, None, 0, 0, reasons), 60),
            None
        );
    }
}
