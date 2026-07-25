use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub(super) struct EdgeRow {
    pub id: i64,
    pub edge_type: String,
    pub edge_trust: String,
    pub from_node_kind: String,
    pub from_node_id: i64,
    pub to_node_kind: String,
    pub to_node_id: i64,
    pub confidence: f64,
}

pub(super) enum GraphTableState {
    Missing,
    Empty,
    Populated,
}

pub(super) fn graph_table_state(conn: &Connection) -> Result<GraphTableState> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'graph_edges'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(GraphTableState::Missing);
    }
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    Ok(if count == 0 {
        GraphTableState::Empty
    } else {
        GraphTableState::Populated
    })
}

pub(super) fn seed_edges(
    conn: &Connection,
    seed_id: i64,
    reference_time_epoch: i64,
    limit: usize,
) -> Result<Vec<EdgeRow>> {
    load_edges(
        conn,
        "((from_node_kind = 'memory' AND from_node_id = ?1) OR \
          (to_node_kind = 'memory' AND to_node_id = ?1))",
        &[&seed_id, &reference_time_epoch, &i64::try_from(limit + 1)?],
    )
}

pub(super) fn bridge_edges(
    conn: &Connection,
    edge_type: &str,
    bridge_kind: &str,
    bridge_id: i64,
    reference_time_epoch: i64,
    limit: usize,
) -> Result<Vec<EdgeRow>> {
    let sql = "SELECT id, edge_type, edge_trust, from_node_kind, from_node_id,
                to_node_kind, to_node_id, COALESCE(confidence, 0.0)
         FROM graph_edges
         WHERE edge_trust = 'trusted'
           AND edge_type = ?1
           AND from_node_kind = 'memory'
           AND to_node_kind = ?2
           AND to_node_id = ?3
           AND (valid_from_epoch IS NULL OR valid_from_epoch <= ?4)
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?4)
         ORDER BY id
         LIMIT ?5";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        rusqlite::params![
            edge_type,
            bridge_kind,
            bridge_id,
            reference_time_epoch,
            i64::try_from(limit + 1)?
        ],
        map_edge,
    )?;
    crate::db::query::collect_rows(rows)
}

fn load_edges(
    conn: &Connection,
    adjacency_predicate: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<Vec<EdgeRow>> {
    let sql = format!(
        "SELECT id, edge_type, edge_trust, from_node_kind, from_node_id,
                to_node_kind, to_node_id, COALESCE(confidence, 0.0)
         FROM graph_edges
         WHERE {adjacency_predicate}
           AND (valid_from_epoch IS NULL OR valid_from_epoch <= ?2)
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?2)
         ORDER BY edge_type, id
         LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params, map_edge)?;
    crate::db::query::collect_rows(rows)
}

fn map_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<EdgeRow> {
    Ok(EdgeRow {
        id: row.get(0)?,
        edge_type: row.get(1)?,
        edge_trust: row.get(2)?,
        from_node_kind: row.get(3)?,
        from_node_id: row.get(4)?,
        to_node_kind: row.get(5)?,
        to_node_id: row.get(6)?,
        confidence: row.get(7)?,
    })
}

pub(super) struct MemoryEligibility<'a> {
    pub project: Option<&'a str>,
    pub memory_type: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub include_inactive: bool,
}

pub(super) fn memory_is_eligible(
    conn: &Connection,
    memory_id: i64,
    filters: MemoryEligibility<'_>,
) -> Result<bool> {
    if memory_id <= 0 {
        bail!("graph target memory id must be positive");
    }
    let mut conditions = vec!["m.id = ?1".to_string()];
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(memory_id)];
    let mut idx = 2;
    conditions.push(crate::memory::memory_current_filter_sql(
        "m.status",
        "m.expires_at_epoch",
        filters.include_inactive,
    ));
    idx = crate::retrieval::memory_search::push_project_filter(
        "m.project",
        filters.project,
        idx,
        &mut conditions,
        &mut values,
    );
    if let Some(branch) = filters.branch {
        conditions.push(format!("(m.branch = ?{idx} OR m.branch IS NULL)"));
        values.push(Box::new(branch.to_string()));
        idx += 1;
    }
    if let Some(memory_type) = filters.memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        values.push(Box::new(memory_type.to_string()));
    }
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM memories m WHERE {})",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&values);
    Ok(conn.query_row(&sql, refs.as_slice(), |row| row.get(0))?)
}
