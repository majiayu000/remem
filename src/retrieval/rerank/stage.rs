//! Shared final-candidate rerank stage.
//!
//! Standard search and SessionStart both call `run_stage` with their already
//! eligible, baseline-ordered candidates. The stage never recalls memories,
//! never bypasses eligibility, and atomically falls back to the complete
//! baseline order on any failure.

use std::time::{Duration, Instant};

use crate::perf::PhaseTiming;

use super::config::RerankConfig;
use super::inventory::{inventory_state, RerankerInventoryState};
use super::model::{score_documents, RerankModelError};
use super::types::{RerankCandidate, RerankDisabledReason, RerankOutcome, RerankStatus};

/// Run the shared rerank stage over eligible baseline-ordered candidates.
///
/// The caller keeps its complete baseline order whenever the returned outcome
/// is `NotApplied`; when `Applied`, `ordered_ids` is the fixed top-k result
/// set (score desc, then baseline rank asc, then id asc, with
/// `verify-before-trust` candidates hard-partitioned last).
pub(super) fn run_stage(
    config: &RerankConfig,
    query: &str,
    candidates: &[RerankCandidate],
) -> RerankOutcome {
    let total_start = Instant::now();
    if !config.enabled {
        return RerankOutcome::not_applied(RerankDisabledReason::Off);
    }
    if query.trim().is_empty() {
        return RerankOutcome::not_applied(RerankDisabledReason::EmptyQuery);
    }
    let mut outcome = RerankOutcome {
        status: RerankStatus::Applied,
        ordered_ids: vec![],
        preset: None,
        model_manifest_sha256: None,
        input_count: 0,
        output_count: 0,
        top_n: config.top_n,
        top_k: config.top_k,
        timings: vec![],
    };
    if candidates.is_empty() {
        // Legal empty candidate set: an empty applied outcome without loading
        // the model and without a pseudo error (B-002).
        outcome
            .timings
            .push(PhaseTiming::elapsed("rerank_total", total_start));
        return outcome;
    }

    let verified = match inventory_state(config) {
        Ok(RerankerInventoryState::Ready(verified)) => verified,
        Ok(state) => {
            let reason = state
                .disabled_reason()
                .unwrap_or(RerankDisabledReason::ModelCorrupt);
            let detail = match state {
                RerankerInventoryState::Missing(detail)
                | RerankerInventoryState::Corrupt(detail) => detail,
                RerankerInventoryState::Ready(_) => unreachable!(),
            };
            log_stage_error(reason, &detail);
            return RerankOutcome::not_applied(reason);
        }
        Err(error) => {
            log_stage_error(RerankDisabledReason::ModelCorrupt, &error.to_string());
            return RerankOutcome::not_applied(RerankDisabledReason::ModelCorrupt);
        }
    };
    outcome.preset = Some(verified.manifest.preset.clone());
    outcome.model_manifest_sha256 = Some(verified.manifest_sha256.clone());

    let top_n: Vec<&RerankCandidate> = candidates.iter().take(config.top_n).collect();
    outcome.input_count = top_n.len();
    let documents: Vec<String> = top_n
        .iter()
        .map(|candidate| truncate_utf8(&candidate.document, config.max_document_bytes))
        .collect();
    let deadline = Instant::now() + Duration::from_millis(config.deadline_ms);
    let report = match score_documents(&verified, query, &documents, deadline) {
        Ok(report) => report,
        Err(error) => {
            let (reason, detail) = match error {
                RerankModelError::Load(error) => {
                    (RerankDisabledReason::ModelLoadFailed, error.to_string())
                }
                RerankModelError::Inference(error) => {
                    (RerankDisabledReason::InferenceFailed, error.to_string())
                }
                RerankModelError::DeadlineExceeded => (
                    RerankDisabledReason::DeadlineExceeded,
                    format!("rerank deadline of {}ms elapsed", config.deadline_ms),
                ),
            };
            log_stage_error(reason, &detail);
            return RerankOutcome::not_applied(reason);
        }
    };
    if let Some(load_ms) = report.load_ms {
        outcome.timings.push(PhaseTiming {
            phase: "rerank_model_load".to_string(),
            elapsed_ms: load_ms,
        });
    }
    outcome.timings.push(PhaseTiming {
        phase: "rerank_inference".to_string(),
        elapsed_ms: report.inference_ms,
    });

    outcome.ordered_ids = order_scored_candidates(&top_n, &report.scores, config.top_k);
    outcome.output_count = outcome.ordered_ids.len();
    outcome
        .timings
        .push(PhaseTiming::elapsed("rerank_total", total_start));
    outcome
}

/// Deterministic final ordering: rerank score desc, baseline rank asc, stable
/// id asc; then the human-approved hard partition places every
/// `verify-before-trust` candidate behind all normal candidates while
/// preserving each partition's internal order; finally the fixed top-k cut.
pub(super) fn order_scored_candidates(
    candidates: &[&RerankCandidate],
    scores: &[f32],
    top_k: usize,
) -> Vec<i64> {
    let mut scored: Vec<(&RerankCandidate, f32)> = candidates
        .iter()
        .zip(scores.iter())
        .map(|(candidate, score)| (*candidate, *score))
        .collect();
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.baseline_rank.cmp(&right.0.baseline_rank))
            .then_with(|| left.0.id.cmp(&right.0.id))
    });
    let (normal, verify_before_trust): (Vec<_>, Vec<_>) = scored
        .into_iter()
        .partition(|(candidate, _)| !candidate.verify_before_trust);
    normal
        .into_iter()
        .chain(verify_before_trust)
        .take(top_k)
        .map(|(candidate, _)| candidate.id)
        .collect()
}

/// Truncate at a UTF-8 character boundary; only affects the model input,
/// never the stored or rendered memory content.
pub(super) fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn log_stage_error(reason: RerankDisabledReason, detail: &str) {
    // B-006/B-007: broken-but-enabled states are fail-visible at error level
    // and fall back to the complete baseline order.
    if reason.is_error() {
        crate::log::error(
            "rerank",
            &format!("rerank not applied (reason={}): {detail}", reason.as_str()),
        );
    }
}
