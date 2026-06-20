use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::types::ToSql;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{DbState, GraphEdgeItem, GraphNodeItem, GraphParams, GraphResponse};

type NodeRow = (i64, String, Option<String>, i64);
const GRAPH_MEMORIES_PER_NODE_LIMIT: i64 = 200;

pub(in crate::api) async fn handle_graph(
    State(_state): State<DbState>,
    Query(params): Query<GraphParams>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let limit = params.limit.unwrap_or(60).clamp(1, 200);
    let requested_project = params
        .project
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let nodes: Vec<NodeRow> = match (|| -> anyhow::Result<Vec<NodeRow>> {
        let mut conditions = vec![crate::memory::memory_current_filter_sql(
            "m.status",
            "m.expires_at_epoch",
            false,
        )];
        conditions.push(crate::memory::memory_not_superseded_filter_sql("m"));
        if !params.include_suppressed.unwrap_or(false) {
            conditions.push(crate::memory::suppression::memory_policy_filter_sql("m"));
        }
        let mut binds: Vec<Box<dyn ToSql>> = Vec::new();
        let mut idx = 1usize;
        if let Some(project) = requested_project.as_deref() {
            push_graph_project_filter(project, &mut idx, &mut conditions, &mut binds);
        }
        let where_sql = conditions.join(" AND ");
        let sql = format!(
            "SELECT e.id, e.canonical_name, e.entity_type, COUNT(DISTINCT me.memory_id) AS mention_count \
             FROM memory_entities me \
             JOIN memories m ON m.id = me.memory_id \
             JOIN entities e ON e.id = me.entity_id \
             WHERE {where_sql} \
             GROUP BY e.id, e.canonical_name, e.entity_type \
             ORDER BY mention_count DESC, e.id ASC LIMIT ?{idx}"
        );
        binds.push(Box::new(limit));
        let mut stmt = conn.prepare(&sql)?;
        let refs = crate::db::to_sql_refs(&binds);
        let rows = stmt.query_map(refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, rusqlite::Error>>()
            .map_err(anyhow::Error::from)
    })() {
        Ok(v) => v,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "nodes_failed",
                &err.to_string(),
            )
            .into_response()
        }
    };

    let mut node_ids: Vec<i64> = nodes.iter().map(|(id, _, _, _)| *id).collect();
    node_ids.sort_unstable();

    let mut ent_mems: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut mem_ents: HashMap<i64, Vec<i64>> = HashMap::new();
    if !node_ids.is_empty() {
        let mut binds: Vec<Box<dyn ToSql>> = Vec::new();
        let mut idx = 1usize;
        let placeholders: Vec<String> = node_ids
            .iter()
            .map(|id| {
                let placeholder = format!("?{idx}");
                binds.push(Box::new(*id) as Box<dyn ToSql>);
                idx += 1;
                placeholder
            })
            .collect();
        let mut conditions = vec![
            format!("me.entity_id IN ({})", placeholders.join(",")),
            crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false),
            crate::memory::memory_not_superseded_filter_sql("m"),
        ];
        if !params.include_suppressed.unwrap_or(false) {
            conditions.push(crate::memory::suppression::memory_policy_filter_sql("m"));
        }
        if let Some(project) = requested_project.as_deref() {
            push_graph_project_filter(project, &mut idx, &mut conditions, &mut binds);
        }
        let where_sql = conditions.join(" AND ");
        let sql = format!(
            "SELECT entity_id, memory_id FROM (
                 SELECT me.entity_id, me.memory_id,
                        ROW_NUMBER() OVER (PARTITION BY me.entity_id ORDER BY me.memory_id DESC) AS rn
                 FROM memory_entities me
                 JOIN memories m ON m.id = me.memory_id
                 WHERE {where_sql}
             ) WHERE rn <= ?{idx}",
        );
        binds.push(Box::new(GRAPH_MEMORIES_PER_NODE_LIMIT));
        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "mem_entity_failed",
                    &err.to_string(),
                )
                .into_response()
            }
        };
        let refs = crate::db::to_sql_refs(&binds);
        let rows = match stmt.query_map(refs.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        }) {
            Ok(r) => r,
            Err(err) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "mem_entity_failed",
                    &err.to_string(),
                )
                .into_response()
            }
        };
        for row in rows {
            let (eid, mid) = match row {
                Ok(row) => row,
                Err(err) => {
                    return error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "mem_entity_failed",
                        &err.to_string(),
                    )
                    .into_response()
                }
            };
            ent_mems.entry(eid).or_default().push(mid);
            mem_ents.entry(mid).or_default().push(eid);
        }
    }

    let mut pair_count: HashMap<(i64, i64), i64> = HashMap::new();
    for ents in mem_ents.values() {
        for i in 0..ents.len() {
            for j in (i + 1)..ents.len() {
                let a = ents[i].min(ents[j]);
                let b = ents[i].max(ents[j]);
                *pair_count.entry((a, b)).or_insert(0) += 1;
            }
        }
    }
    let mut edges: Vec<GraphEdgeItem> = pair_count
        .into_iter()
        .map(|((a, b), w)| GraphEdgeItem { a, b, w })
        .collect();
    edges.sort_by_key(|edge| (-edge.w, edge.a, edge.b));
    let edges = edges.into_iter().take((limit as usize) * 3).collect();

    let nodes_out: Vec<GraphNodeItem> = nodes
        .into_iter()
        .map(|(id, name, entity_type, mention_count)| GraphNodeItem {
            id,
            name,
            entity_type,
            mention_count,
            mems: ent_mems.remove(&id).unwrap_or_default(),
        })
        .collect();

    Json(GraphResponse {
        nodes: nodes_out,
        edges,
    })
    .into_response()
}

fn push_graph_project_filter(
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    binds: &mut Vec<Box<dyn ToSql>>,
) {
    let owner_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    let target_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    let legacy_idx = *idx;
    binds.push(Box::new(project.to_string()));
    *idx += 1;
    conditions.push(format!(
        "((m.owner_scope = 'repo' AND m.owner_key = ?{owner_idx}) \
          OR (m.owner_scope = 'repo' AND m.target_project = ?{target_idx}) \
          OR (m.owner_scope IS NULL AND m.project = ?{legacy_idx}) \
          OR m.scope = 'global' \
          OR (m.owner_scope = 'user' AND m.owner_key = 'user:default'))"
    ));
}
