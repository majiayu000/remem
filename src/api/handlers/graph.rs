use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::{params, types::ToSql};

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

    let nodes: Vec<NodeRow> = match (|| -> anyhow::Result<Vec<NodeRow>> {
        let mut stmt = conn.prepare(
            "SELECT id, canonical_name, entity_type, mention_count \
             FROM entities ORDER BY mention_count DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
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

    let node_ids: HashSet<i64> = nodes.iter().map(|(id, _, _, _)| *id).collect();

    let mut ent_mems: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut mem_ents: HashMap<i64, Vec<i64>> = HashMap::new();
    if !node_ids.is_empty() {
        let placeholders: Vec<String> = (1..=node_ids.len()).map(|i| format!("?{i}")).collect();
        let current_filter =
            crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false);
        let sql = format!(
            "SELECT entity_id, memory_id FROM (
                 SELECT me.entity_id, me.memory_id,
                        ROW_NUMBER() OVER (PARTITION BY me.entity_id ORDER BY me.memory_id DESC) AS rn
                 FROM memory_entities me
                 JOIN memories m ON m.id = me.memory_id
                 WHERE me.entity_id IN ({}) AND {current_filter}
             ) WHERE rn <= ?{}",
            placeholders.join(","),
            node_ids.len() + 1,
        );
        let mut binds: Vec<Box<dyn ToSql>> = node_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn ToSql>)
            .collect();
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
    edges.sort_by_key(|edge| Reverse(edge.w));
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
