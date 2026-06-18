use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::params;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{CandidateItem, CandidateParams, DbState, ListMeta, ListResponse};

pub(in crate::api) async fn handle_list_candidates(
    State(_state): State<DbState>,
    Query(params): Query<CandidateParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let status = params
        .status
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("pending_review");
    let limit = params.limit.unwrap_or(50).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    let total: i64 = match conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates WHERE review_status = ?1",
        params![status],
        |row| row.get(0),
    ) {
        Ok(n) => n,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "count_failed",
                &err.to_string(),
            )
            .into_response()
        }
    };

    let result = (|| -> anyhow::Result<Vec<CandidateItem>> {
        let mut stmt = conn.prepare(
            "SELECT id, memory_type, text, scope, confidence, risk_class, review_status, \
             evidence_event_ids, created_at_epoch FROM memory_candidates \
             WHERE review_status = ?1 ORDER BY created_at_epoch DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![status, limit, offset], |row| {
            let evidence_json: Option<String> = row.get(7)?;
            let evidence_count = evidence_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
                .map(|v| v.len() as i64)
                .unwrap_or(0);
            Ok(CandidateItem {
                id: row.get(0)?,
                memory_type: row.get(1)?,
                text: row.get(2)?,
                scope: row.get(3)?,
                confidence: row.get(4)?,
                risk_class: row.get(5)?,
                review_status: row.get(6)?,
                evidence_count,
                created_at_epoch: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, rusqlite::Error>>()
            .map_err(anyhow::Error::from)
    })();

    let items = match result {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "list_failed",
                &err.to_string(),
            )
            .into_response()
        }
    };

    let count = items.len();
    Json(ListResponse {
        data: items,
        meta: ListMeta {
            count,
            total,
            limit,
            offset,
        },
    })
    .into_response()
}
