use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::types::ToSql;

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

    let mut conditions = vec!["c.review_status = ?1".to_string()];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(status.to_string())];
    let mut idx = 2usize;
    if let Some(project) = params.project.as_deref().filter(|s| !s.is_empty()) {
        push_candidate_project_filter(project, &mut idx, &mut conditions, &mut binds);
    }
    let where_sql = conditions.join(" AND ");

    let count_sql = format!(
        "SELECT COUNT(*) FROM memory_candidates c \
         LEFT JOIN projects p ON p.id = c.project_id \
         WHERE {where_sql}"
    );
    let binds_refs = crate::db::to_sql_refs(&binds);
    let total: i64 = match conn.query_row(&count_sql, binds_refs.as_slice(), |row| row.get(0)) {
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
        let sql = format!(
            "SELECT c.id, c.memory_type, c.text, c.scope, c.confidence, c.risk_class, \
                    c.review_status, c.evidence_event_ids, c.created_at_epoch, \
                    COALESCE(c.target_project, p.project_path, c.source_project) AS project \
             FROM memory_candidates c \
             LEFT JOIN projects p ON p.id = c.project_id \
             WHERE {where_sql} ORDER BY c.created_at_epoch DESC LIMIT ?{idx} OFFSET ?{}",
            idx + 1,
        );
        binds.push(Box::new(limit));
        binds.push(Box::new(offset));
        let mut stmt = conn.prepare(&sql)?;
        let binds_refs2 = crate::db::to_sql_refs(&binds);
        let rows = stmt.query_map(binds_refs2.as_slice(), |row| {
            let evidence_json: Option<String> = row.get(7)?;
            let evidence_count = evidence_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
                .map(|v| v.len() as i64)
                .unwrap_or(0);
            Ok(CandidateItem {
                id: row.get(0)?,
                project: row.get(9)?,
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

fn push_candidate_project_filter(
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    binds: &mut Vec<Box<dyn ToSql>>,
) {
    let project_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    let source_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    let target_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    let owner_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    conditions.push(format!(
        "(p.project_path = ?{project_idx} \
          OR c.source_project = ?{source_idx} \
          OR c.target_project = ?{target_idx} \
          OR (c.owner_scope = 'repo' AND c.owner_key = ?{owner_idx}))"
    ));
}
