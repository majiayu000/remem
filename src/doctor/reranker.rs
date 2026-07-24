use super::types::{Check, Status};

/// Reranker doctor matrix (GH-851):
/// off => OK (explicitly disabled, stable reason), enabled+verified => OK,
/// enabled+missing/corrupt => Fail. Never downloads or loads the model.
pub(super) fn check_reranker() -> Vec<Check> {
    let report = match crate::retrieval::rerank::reranker_status() {
        Ok(report) => report,
        Err(error) => {
            return vec![Check::new(
                "Reranker",
                Status::Fail,
                format!("rerank config invalid: {error}"),
            )];
        }
    };
    if !report.enabled {
        return vec![Check::new(
            "Reranker",
            Status::Ok,
            "rerank=off; second-stage rerank is explicitly disabled",
        )];
    }
    let check = match report.state.as_str() {
        "ready" => Check::new(
            "Reranker",
            Status::Ok,
            format!(
                "rerank enabled preset={} model={} manifest_sha256={}",
                report.preset,
                report.model_id,
                report.manifest_sha256.as_deref().unwrap_or("unknown")
            ),
        ),
        "missing" => Check::new(
            "Reranker",
            Status::Fail,
            format!(
                "rerank enabled but model is missing (reason={}): {}",
                report.disabled_reason.as_deref().unwrap_or("model_missing"),
                report.detail.as_deref().unwrap_or("manifest not installed")
            ),
        ),
        _ => Check::new(
            "Reranker",
            Status::Fail,
            format!(
                "rerank enabled but model failed verification (reason={}): {}",
                report.disabled_reason.as_deref().unwrap_or("model_corrupt"),
                report.detail.as_deref().unwrap_or("verification failed")
            ),
        ),
    };
    vec![check]
}
