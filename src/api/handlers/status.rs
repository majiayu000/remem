use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::Value;

use crate::db;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{DbState, StatusCache, StatusCacheEntry, StatusParams};

const STATUS_CACHE_TTL_SECS: i64 = 2;
const STATUS_CACHE_MAX_STALE_SECS: i64 = 10;
type StatusComputeResult = Result<Value, Box<Response>>;

pub(in crate::api) async fn handle_status(
    State(_state): State<DbState>,
    Query(params): Query<StatusParams>,
    Extension(cache): Extension<StatusCache>,
) -> Response {
    let refresh = params.refresh.unwrap_or(false);
    status_response(&cache, refresh, chrono::Utc::now().timestamp())
}

fn status_response(cache: &StatusCache, refresh: bool, now_epoch: i64) -> Response {
    status_response_with_compute(cache, refresh, now_epoch, compute_status_payload)
}

fn status_response_with_compute<F>(
    cache: &StatusCache,
    refresh: bool,
    now_epoch: i64,
    compute: F,
) -> Response
where
    F: FnOnce() -> StatusComputeResult,
{
    if !refresh {
        if let Some(entry) = fresh_cache_entry(cache, now_epoch) {
            return Json(with_cache_metadata(
                entry.payload,
                true,
                false,
                entry.generated_at_epoch,
                None,
            ))
            .into_response();
        }
    }

    match compute() {
        Ok(payload) => {
            let entry = StatusCacheEntry {
                generated_at_epoch: now_epoch,
                payload: payload.clone(),
            };
            replace_cache_entry(cache, entry);
            Json(with_cache_metadata(payload, false, false, now_epoch, None)).into_response()
        }
        Err(response) => {
            if let Some(entry) = stale_cache_entry(cache, now_epoch) {
                return Json(with_cache_metadata(
                    entry.payload,
                    false,
                    true,
                    entry.generated_at_epoch,
                    Some((
                        "status_refresh_failed",
                        "Status refresh failed; serving bounded stale cached status.",
                    )),
                ))
                .into_response();
            }
            *response
        }
    }
}

fn compute_status_payload() -> StatusComputeResult {
    let conn = open_request_db().map_err(Box::new)?;
    let stats = match db::query_system_stats(&conn) {
        Ok(stats) => stats,
        Err(err) => {
            return Err(Box::new(
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "status_failed",
                    &err.to_string(),
                )
                .into_response(),
            ));
        }
    };

    Ok(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "memories": stats.active_memories,
        "observations": stats.active_observations,
        "total_observations": stats.total_observations,
        "captured_events": stats.captured_events,
        "capture_drop_events": stats.capture_drop_events,
        "unrecovered_capture_spills": stats.unrecovered_capture_spills,
        "pending_extraction_tasks": stats.pending_extraction_tasks,
        "pending_memory_candidates": stats.pending_memory_candidates,
        "pending_graph_candidates": stats.pending_graph_candidates,
        "promotion_funnel": {
            "captured_events": stats.captured_events,
            "observations": stats.total_observations,
            "observation_rate_percent": percent(stats.total_observations, stats.captured_events),
            "candidates": stats.total_memory_candidates,
            "candidate_rate_percent": percent(stats.total_memory_candidates, stats.total_observations),
            "promoted": stats.promoted_memory_candidates,
            "promoted_rate_percent": percent(stats.promoted_memory_candidates, stats.total_memory_candidates),
            "pending_review": stats.pending_review_memory_candidates,
            "pending_review_rate_percent": percent(stats.pending_review_memory_candidates, stats.total_memory_candidates),
        },
    }))
}

fn fresh_cache_entry(cache: &StatusCache, now_epoch: i64) -> Option<StatusCacheEntry> {
    cache
        .entry
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .filter(|entry| now_epoch.saturating_sub(entry.generated_at_epoch) <= STATUS_CACHE_TTL_SECS)
}

fn stale_cache_entry(cache: &StatusCache, now_epoch: i64) -> Option<StatusCacheEntry> {
    cache
        .entry
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .filter(|entry| {
            now_epoch.saturating_sub(entry.generated_at_epoch) <= STATUS_CACHE_MAX_STALE_SECS
        })
}

fn replace_cache_entry(cache: &StatusCache, entry: StatusCacheEntry) {
    if let Ok(mut guard) = cache.entry.lock() {
        *guard = Some(entry);
    } else {
        crate::log::error(
            "api",
            "status cache lock poisoned; status response not cached",
        );
    }
}

fn with_cache_metadata(
    mut payload: Value,
    hit: bool,
    stale: bool,
    generated_at_epoch: i64,
    warning: Option<(&str, &str)>,
) -> Value {
    if let Value::Object(ref mut object) = payload {
        object.insert(
            "cache".to_string(),
            serde_json::json!({
                "hit": hit,
                "stale": stale,
                "generated_at_epoch": generated_at_epoch,
                "ttl_secs": STATUS_CACHE_TTL_SECS,
            }),
        );
        if let Some((code, message)) = warning {
            object.insert(
                "warnings".to_string(),
                serde_json::json!([{
                    "code": code,
                    "message": message,
                }]),
            );
        }
    }
    payload
}

fn percent(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        (numerator as f64 * 100.0) / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};
    use serde_json::{json, Value};

    use super::{status_response_with_compute, StatusCacheEntry};
    use crate::api::helpers::error_response;
    use crate::api::types::StatusCache;

    #[tokio::test]
    async fn stale_cache_response_marks_warning() -> anyhow::Result<()> {
        let cache = StatusCache::default();
        {
            let mut guard = cache
                .entry
                .lock()
                .map_err(|_| anyhow::anyhow!("cache lock"))?;
            *guard = Some(StatusCacheEntry {
                generated_at_epoch: 100,
                payload: json!({"version": "test", "memories": 1}),
            });
        }

        let response = status_response_with_compute(&cache, true, 105, || {
            Err(Box::new(
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "boom", "failed").into_response(),
            ))
        });
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .context("body should read")?;
        let payload: Value = serde_json::from_slice(&body).context("json body")?;

        assert_eq!(payload["cache"]["hit"], false);
        assert_eq!(payload["cache"]["stale"], true);
        assert_eq!(payload["cache"]["generated_at_epoch"], 100);
        assert_eq!(payload["warnings"][0]["code"], "status_refresh_failed");
        Ok(())
    }

    #[tokio::test]
    async fn failed_refresh_without_stale_cache_returns_error() -> anyhow::Result<()> {
        let cache = StatusCache::default();

        let response = status_response_with_compute(&cache, true, 105, || {
            Err(Box::new(
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "boom", "failed").into_response(),
            ))
        });
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .context("body should read")?;
        let payload: Value = serde_json::from_slice(&body).context("json body")?;

        assert_eq!(payload["error"]["code"], "boom");
        assert!(payload.get("cache").is_none());
        assert!(payload.get("warnings").is_none());
        Ok(())
    }
}
