use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEdgeType {
    Supersedes,
    Duplicates,
    Conflicts,
    DerivedFrom,
    MergedInto,
    SplitFrom,
}

impl MemoryEdgeType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::Duplicates => "duplicates",
            Self::Conflicts => "conflicts",
            Self::DerivedFrom => "derived_from",
            Self::MergedInto => "merged_into",
            Self::SplitFrom => "split_from",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryEdgeWriteContext<'a> {
    pub state_key_id: Option<i64>,
    pub source_candidate_id: Option<i64>,
    pub evidence_event_ids: &'a [i64],
    pub source_operation_id: Option<i64>,
    pub confidence: Option<f64>,
    pub reason: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEdgeInput<'a> {
    pub edge_type: MemoryEdgeType,
    pub from_memory_id: Option<i64>,
    pub to_memory_id: Option<i64>,
    pub state_key_id: Option<i64>,
    pub source_candidate_id: Option<i64>,
    pub evidence_event_ids: &'a [i64],
    pub source_operation_id: Option<i64>,
    pub confidence: Option<f64>,
    pub reason: Option<&'a str>,
}

pub fn insert_memory_edge(conn: &Connection, input: &MemoryEdgeInput<'_>) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let evidence_event_ids = if input.evidence_event_ids.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(input.evidence_event_ids)
                .context("serialize memory edge evidence event ids")?,
        )
    };
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, source_candidate_id,
          evidence_event_ids, source_operation_id, confidence, reason, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            input.edge_type.as_str(),
            input.from_memory_id,
            input.to_memory_id,
            input.state_key_id,
            input.source_candidate_id,
            evidence_event_ids.as_deref(),
            input.source_operation_id,
            input.confidence,
            input.reason,
            now
        ],
    )
    .context("insert memory edge")?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_replacement_edges(
    conn: &Connection,
    edge_type: MemoryEdgeType,
    from_memory_ids: &[i64],
    to_memory_id: i64,
    context: MemoryEdgeWriteContext<'_>,
) -> Result<usize> {
    let state_key_id = context
        .state_key_id
        .or(memory_state_key_id(conn, to_memory_id)?);
    let mut seen = std::collections::HashSet::with_capacity(from_memory_ids.len());
    let mut inserted = 0usize;
    for from_memory_id in from_memory_ids
        .iter()
        .copied()
        .filter(|id| *id != to_memory_id && seen.insert(*id))
    {
        insert_memory_edge(
            conn,
            &MemoryEdgeInput {
                edge_type,
                from_memory_id: Some(from_memory_id),
                to_memory_id: Some(to_memory_id),
                state_key_id,
                source_candidate_id: context.source_candidate_id,
                evidence_event_ids: context.evidence_event_ids,
                source_operation_id: context.source_operation_id,
                confidence: context.confidence,
                reason: context.reason,
            },
        )?;
        inserted += 1;
    }
    Ok(inserted)
}

pub fn insert_supersedes_edges(
    conn: &Connection,
    from_memory_ids: &[i64],
    to_memory_id: i64,
    context: MemoryEdgeWriteContext<'_>,
) -> Result<usize> {
    insert_replacement_edges(
        conn,
        MemoryEdgeType::Supersedes,
        from_memory_ids,
        to_memory_id,
        context,
    )
}

pub fn insert_merged_into_edges(
    conn: &Connection,
    from_memory_ids: &[i64],
    to_memory_id: i64,
    context: MemoryEdgeWriteContext<'_>,
) -> Result<usize> {
    insert_replacement_edges(
        conn,
        MemoryEdgeType::MergedInto,
        from_memory_ids,
        to_memory_id,
        context,
    )
}

pub fn insert_conflicts_edges(
    conn: &Connection,
    from_memory_ids: &[i64],
    to_memory_id: i64,
    context: MemoryEdgeWriteContext<'_>,
) -> Result<usize> {
    insert_replacement_edges(
        conn,
        MemoryEdgeType::Conflicts,
        from_memory_ids,
        to_memory_id,
        context,
    )
}

pub fn insert_pairwise_conflict_edges(
    conn: &Connection,
    memory_ids: &[i64],
    context: MemoryEdgeWriteContext<'_>,
) -> Result<usize> {
    let mut ids = memory_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();

    let mut inserted = 0usize;
    for (idx, from_memory_id) in ids.iter().copied().enumerate() {
        for to_memory_id in ids.iter().copied().skip(idx + 1) {
            for state_key_id in
                conflict_edge_state_keys(conn, from_memory_id, to_memory_id, context.state_key_id)?
            {
                insert_memory_edge(
                    conn,
                    &MemoryEdgeInput {
                        edge_type: MemoryEdgeType::Conflicts,
                        from_memory_id: Some(from_memory_id),
                        to_memory_id: Some(to_memory_id),
                        state_key_id,
                        source_candidate_id: context.source_candidate_id,
                        evidence_event_ids: context.evidence_event_ids,
                        source_operation_id: context.source_operation_id,
                        confidence: context.confidence,
                        reason: context.reason,
                    },
                )?;
                inserted += 1;
            }
        }
    }
    Ok(inserted)
}

fn conflict_edge_state_keys(
    conn: &Connection,
    from_memory_id: i64,
    to_memory_id: i64,
    explicit_state_key_id: Option<i64>,
) -> Result<Vec<Option<i64>>> {
    if explicit_state_key_id.is_some() {
        return Ok(vec![explicit_state_key_id]);
    }
    let from_state_key_id = memory_state_key_id(conn, from_memory_id)?;
    let to_state_key_id = memory_state_key_id(conn, to_memory_id)?;
    if from_state_key_id == to_state_key_id {
        return Ok(vec![from_state_key_id]);
    }
    let mut state_key_ids = Vec::new();
    if from_state_key_id.is_some() {
        state_key_ids.push(from_state_key_id);
    }
    if to_state_key_id.is_some() {
        state_key_ids.push(to_state_key_id);
    }
    if state_key_ids.is_empty() {
        state_key_ids.push(None);
    }
    Ok(state_key_ids)
}

fn memory_state_key_id(conn: &Connection, memory_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT state_key_id FROM memories WHERE id = ?1",
            [memory_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .with_context(|| format!("load state_key_id for memory edge target id={memory_id}"))?
        .flatten())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemoryEdgeSummary {
    pub incoming_count: usize,
    pub outgoing_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incoming: Vec<MemoryEdgeReference>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outgoing: Vec<MemoryEdgeReference>,
}

impl MemoryEdgeSummary {
    pub fn has_edges(&self) -> bool {
        self.incoming_count > 0 || self.outgoing_count > 0
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemoryEdgeReference {
    pub id: i64,
    pub edge_type: String,
    pub from_memory_id: Option<i64>,
    pub to_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_key_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence_event_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at_epoch: i64,
}

pub fn load_memory_edge_summary(conn: &Connection, memory_id: i64) -> Result<MemoryEdgeSummary> {
    let incoming_count = count_edges(conn, "to_memory_id", memory_id)?;
    let outgoing_count = count_edges(conn, "from_memory_id", memory_id)?;
    Ok(MemoryEdgeSummary {
        incoming_count,
        outgoing_count,
        incoming: load_edge_refs(conn, "to_memory_id", memory_id)?,
        outgoing: load_edge_refs(conn, "from_memory_id", memory_id)?,
    })
}

fn count_edges(conn: &Connection, column: &str, memory_id: i64) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM memory_edges WHERE {column} = ?1");
    let count: i64 = conn.query_row(&sql, [memory_id], |row| row.get(0))?;
    Ok(count as usize)
}

fn load_edge_refs(
    conn: &Connection,
    column: &str,
    memory_id: i64,
) -> Result<Vec<MemoryEdgeReference>> {
    let sql = format!(
        "SELECT id, edge_type, from_memory_id, to_memory_id, state_key_id,
                source_candidate_id, evidence_event_ids, source_operation_id,
                confidence, reason, created_at_epoch
         FROM memory_edges
         WHERE {column} = ?1
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT 25"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([memory_id], |row| {
        let evidence_json: Option<String> = row.get(6)?;
        let evidence_event_ids = match evidence_json {
            Some(json) => serde_json::from_str::<Vec<i64>>(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })?,
            None => Vec::new(),
        };
        Ok(MemoryEdgeReference {
            id: row.get(0)?,
            edge_type: row.get(1)?,
            from_memory_id: row.get(2)?,
            to_memory_id: row.get(3)?,
            state_key_id: row.get(4)?,
            source_candidate_id: row.get(5)?,
            evidence_event_ids,
            source_operation_id: row.get(7)?,
            confidence: row.get(8)?,
            reason: row.get(9)?,
            created_at_epoch: row.get(10)?,
        })
    })?;
    crate::db::query::collect_rows(rows).context("load memory edge references")
}
