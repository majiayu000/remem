use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::types::ToSql;

use super::super::helpers::{error_response, memory_to_item_with_conn, open_request_db};
use super::super::types::{DbState, ListMeta, ListParams, ListResponse, MemoryItem};

pub(in crate::api) async fn handle_list_memories(
    State(_state): State<DbState>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let limit = params.limit.unwrap_or(50).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<Box<dyn ToSql>> = Vec::new();
    let mut idx = 1usize;

    if let Some(p) = params.project.as_deref().filter(|s| !s.is_empty()) {
        conditions.push(format!("(project = ?{idx} OR scope = 'global')"));
        binds.push(Box::new(p.to_string()));
        idx += 1;
    }
    if let Some(t) = params.memory_type.as_deref().filter(|s| !s.is_empty()) {
        conditions.push(format!("memory_type = ?{idx}"));
        binds.push(Box::new(t.to_string()));
        idx += 1;
    }
    if let Some(s) = params.scope.as_deref().filter(|s| !s.is_empty()) {
        conditions.push(format!("scope = ?{idx}"));
        binds.push(Box::new(s.to_string()));
        idx += 1;
    }
    if let Some(s) = params.status.as_deref().filter(|s| !s.is_empty()) {
        if s == "active" {
            conditions.push(format!(
                "({})",
                crate::memory::memory_current_filter_sql("status", "expires_at_epoch", false)
            ));
        } else {
            conditions.push(format!("status = ?{idx}"));
            binds.push(Box::new(s.to_string()));
            idx += 1;
        }
    }
    if let Some(b) = params.branch.as_deref().filter(|s| !s.is_empty()) {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        binds.push(Box::new(b.to_string()));
        idx += 1;
    }
    if let Some(q) = params.q.as_deref().filter(|s| !s.is_empty()) {
        conditions.push(format!("(title LIKE ?{idx} OR content LIKE ?{idx})"));
        binds.push(Box::new(format!("%{q}%")));
        idx += 1;
    }

    let where_sql = if conditions.is_empty() {
        "1=1".to_string()
    } else {
        conditions.join(" AND ")
    };

    let binds_refs = crate::db::to_sql_refs(&binds);
    let total: i64 = match conn.query_row(
        &format!("SELECT COUNT(*) FROM memories WHERE {where_sql}"),
        binds_refs.as_slice(),
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

    let sql = format!(
        "SELECT {} FROM memories WHERE {} ORDER BY updated_at_epoch DESC LIMIT ?{idx} OFFSET ?{}",
        crate::memory::types::MEMORY_COLS,
        where_sql,
        idx + 1,
    );
    binds.push(Box::new(limit));
    binds.push(Box::new(offset));

    let result = (|| -> anyhow::Result<Vec<MemoryItem>> {
        let mut stmt = conn.prepare(&sql)?;
        let binds_refs2 = crate::db::to_sql_refs(&binds);
        let rows = stmt.query_map(
            binds_refs2.as_slice(),
            crate::memory::types::map_memory_row_pub,
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(memory_to_item_with_conn(&conn, &row?));
        }
        Ok(out)
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
