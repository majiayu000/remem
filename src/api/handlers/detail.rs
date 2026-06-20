use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::params;

use crate::memory;

use super::super::helpers::{
    error_response, memory_to_item_with_conn, open_request_db, staleness_error_response,
};
use super::super::types::MemoryDetailParams;
use super::super::types::{DbState, MemoryDetailResponse, MemoryEdgeItem};

pub(in crate::api) async fn handle_memory_detail(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
    Query(params): Query<MemoryDetailParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let policy_filter = if params.include_suppressed.unwrap_or(false) {
        "1=1".to_string()
    } else {
        crate::memory::suppression::memory_policy_filter_sql("memories")
    };
    let mem = match conn.query_row(
        &format!(
            "SELECT {} FROM memories WHERE id = ?1 AND {}",
            crate::memory::types::MEMORY_COLS,
            policy_filter
        ),
        params![id],
        crate::memory::types::map_memory_row_pub,
    ) {
        Ok(m) => m,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                &format!("memory {id} not found"),
            )
            .into_response()
        }
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "query_failed",
                &err.to_string(),
            )
            .into_response()
        }
    };

    let result = (|| -> anyhow::Result<(Vec<String>, Vec<MemoryEdgeItem>)> {
        let mut stmt = conn.prepare(
            "SELECT e.canonical_name FROM memory_entities me \
             JOIN entities e ON e.id = me.entity_id \
             WHERE me.memory_id = ?1 ORDER BY e.mention_count DESC",
        )?;
        let entities: Vec<String> = stmt
            .query_map(params![id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, edge_type, from_memory_id, to_memory_id, confidence FROM memory_edges \
             WHERE from_memory_id = ?1 OR to_memory_id = ?1 ORDER BY created_at_epoch DESC, id DESC",
        )?;
        let edges: Vec<MemoryEdgeItem> = stmt
            .query_map(params![id], |row| {
                Ok(MemoryEdgeItem {
                    id: row.get(0)?,
                    edge_type: row.get(1)?,
                    from_memory_id: row.get(2)?,
                    to_memory_id: row.get(3)?,
                    confidence: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok((entities, edges))
    })();

    let (entities, edges) = match result {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "detail_failed",
                &err.to_string(),
            )
            .into_response()
        }
    };

    let memory = match memory_to_item_with_conn(&conn, &mem) {
        Ok(item) => item,
        Err(err) => return staleness_error_response(&err).into_response(),
    };

    if let Err(err) = memory::mark_memories_accessed(&conn, &[id]) {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_error",
            &err.to_string(),
        )
        .into_response();
    }

    Json(MemoryDetailResponse {
        memory,
        entities,
        edges,
    })
    .into_response()
}
