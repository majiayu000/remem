use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::params;

use super::super::helpers::{error_response, memory_to_item_with_conn, open_request_db};
use super::super::types::{DbState, MemoryDetailResponse, MemoryEdgeItem};

pub(in crate::api) async fn handle_memory_detail(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let mem = match conn.query_row(
        &format!(
            "SELECT {} FROM memories WHERE id = ?1",
            crate::memory::types::MEMORY_COLS
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
            .filter_map(|r| r.ok())
            .collect();

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
            .filter_map(|r| r.ok())
            .collect();
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

    Json(MemoryDetailResponse {
        memory: memory_to_item_with_conn(&conn, &mem),
        entities,
        edges,
    })
    .into_response()
}
