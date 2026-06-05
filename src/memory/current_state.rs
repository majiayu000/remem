use anyhow::{bail, Context, Result};
use rusqlite::{types::ToSql, Connection, OptionalExtension};
use serde::Serialize;

use crate::memory::{self, Memory};

const HISTORY_LIMIT: i64 = 10;

#[derive(Debug, Clone, Default)]
pub struct CurrentStateRequest {
    pub state_key: String,
    pub project: Option<String>,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub memory_type: Option<String>,
    pub as_of_epoch: Option<i64>,
    pub include_history: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateResult {
    pub status: String,
    pub state_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<CurrentStateKeySummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<CurrentStateKeySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<CurrentStateAnswer>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<CurrentStateMemoryRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<CurrentStateMemoryRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<CurrentStateFact>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<CurrentStateWhy>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateKeySummary {
    pub id: i64,
    pub owner_scope: String,
    pub owner_key: String,
    pub memory_type: String,
    pub state_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_label: Option<String>,
    pub state_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_memory_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateAnswer {
    pub id: i64,
    pub title: String,
    pub text: String,
    pub memory_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub project: String,
    pub scope: String,
    pub status: String,
    pub updated_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateMemoryRef {
    pub id: i64,
    pub title: String,
    pub memory_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    pub project: String,
    pub status: String,
    pub updated_at_epoch: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence_event_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateWhy {
    pub edge_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence_event_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_candidate_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<i64>,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CurrentStateFact {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_memory_id: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_event_ids: Vec<i64>,
    pub status: String,
}

pub fn current_state(conn: &Connection, req: &CurrentStateRequest) -> Result<CurrentStateResult> {
    let state_key = req.state_key.trim();
    if state_key.is_empty() {
        bail!("state_key is required");
    }

    let matches = load_state_key_matches(conn, req, state_key)?;
    if matches.is_empty() {
        return Ok(empty_result("not_found", state_key, req.as_of_epoch));
    }
    if matches.len() > 1 {
        return Ok(CurrentStateResult {
            status: "ambiguous".to_string(),
            state_key: state_key.to_string(),
            as_of_epoch: req.as_of_epoch,
            state: None,
            matches,
            current: None,
            conflicts: Vec::new(),
            history: Vec::new(),
            facts: Vec::new(),
            why: Vec::new(),
        });
    }

    let state = matches[0].clone();
    let (status, current, conflicts) = if let Some(as_of_epoch) = req.as_of_epoch {
        resolve_as_of_state(conn, state.id, as_of_epoch)?
    } else {
        resolve_current_state(conn, &state)?
    };

    let history = if req.include_history {
        current
            .as_ref()
            .map(|current| load_history(conn, current.id))
            .transpose()?
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let why = current
        .as_ref()
        .map(|current| load_why(conn, current.id))
        .transpose()?
        .unwrap_or_default();
    let facts = current
        .as_ref()
        .map(|current| load_facts_for_memory(conn, current.id, req.as_of_epoch))
        .transpose()?
        .unwrap_or_default();

    Ok(CurrentStateResult {
        status,
        state_key: state_key.to_string(),
        as_of_epoch: req.as_of_epoch,
        state: Some(state),
        matches: Vec::new(),
        current,
        conflicts,
        history,
        facts,
        why,
    })
}

fn empty_result(status: &str, state_key: &str, as_of_epoch: Option<i64>) -> CurrentStateResult {
    CurrentStateResult {
        status: status.to_string(),
        state_key: state_key.to_string(),
        as_of_epoch,
        state: None,
        matches: Vec::new(),
        current: None,
        conflicts: Vec::new(),
        history: Vec::new(),
        facts: Vec::new(),
        why: Vec::new(),
    }
}

fn load_state_key_matches(
    conn: &Connection,
    req: &CurrentStateRequest,
    state_key: &str,
) -> Result<Vec<CurrentStateKeySummary>> {
    let mut conditions = vec!["state_key = ?1".to_string()];
    let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(state_key.to_string())];
    let mut idx = 2;

    if let Some(memory_type) = req
        .memory_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        conditions.push(format!("memory_type = ?{idx}"));
        params_vec.push(Box::new(memory_type.to_string()));
        idx += 1;
    }

    match (
        req.owner_scope
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        req.owner_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    ) {
        (Some(owner_scope), Some(owner_key)) => {
            conditions.push(format!("owner_scope = ?{idx}"));
            params_vec.push(Box::new(owner_scope.to_string()));
            idx += 1;
            conditions.push(format!("owner_key = ?{idx}"));
            params_vec.push(Box::new(owner_key.to_string()));
        }
        (Some(_), None) | (None, Some(_)) => {
            bail!("owner_scope and owner_key must be provided together");
        }
        (None, None) => {
            if let Some(project) = req
                .project
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                conditions.push(format!(
                    "((owner_scope = 'repo' AND owner_key = ?{idx})
                       OR (owner_scope = 'user' AND owner_key = 'user:default'))"
                ));
                params_vec.push(Box::new(project.to_string()));
            }
        }
    }

    let sql = format!(
        "SELECT id, owner_scope, owner_key, memory_type, state_key, state_label,
                state_status, current_memory_id
         FROM memory_state_keys
         WHERE {}
         ORDER BY
             CASE owner_scope WHEN 'repo' THEN 0 WHEN 'user' THEN 1 ELSE 2 END,
             updated_at_epoch DESC,
             id DESC
         LIMIT 3",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&params_vec);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(CurrentStateKeySummary {
            id: row.get(0)?,
            owner_scope: row.get(1)?,
            owner_key: row.get(2)?,
            memory_type: row.get(3)?,
            state_key: row.get(4)?,
            state_label: row.get(5)?,
            state_status: row.get(6)?,
            current_memory_id: row.get(7)?,
        })
    })?;
    crate::db::query::collect_rows(rows).context("load current-state key matches")
}

fn resolve_current_state(
    conn: &Connection,
    state: &CurrentStateKeySummary,
) -> Result<(
    String,
    Option<CurrentStateAnswer>,
    Vec<CurrentStateMemoryRef>,
)> {
    if state.state_status != "active" {
        return Ok(("no_current".to_string(), None, Vec::new()));
    }

    let now_epoch = chrono::Utc::now().timestamp();
    let current = state
        .current_memory_id
        .map(|id| load_active_memory(conn, id, now_epoch))
        .transpose()?
        .flatten()
        .map(CurrentStateAnswer::from_memory);
    let conflicts = match current.as_ref() {
        Some(current) => load_active_state_key_rivals(conn, state.id, current.id, now_epoch)?,
        None => load_active_state_key_rivals(conn, state.id, -1, now_epoch)?,
    };

    let status = if !conflicts.is_empty() {
        "unresolved_conflict"
    } else if current.is_some() {
        "current"
    } else {
        "no_current"
    };
    Ok((status.to_string(), current, conflicts))
}

fn resolve_as_of_state(
    conn: &Connection,
    state_key_id: i64,
    as_of_epoch: i64,
) -> Result<(
    String,
    Option<CurrentStateAnswer>,
    Vec<CurrentStateMemoryRef>,
)> {
    let candidates = load_memories_as_of(conn, state_key_id, as_of_epoch)?;
    match candidates.as_slice() {
        [] => Ok(("no_current".to_string(), None, Vec::new())),
        [memory] => Ok((
            "current".to_string(),
            Some(CurrentStateAnswer::from_memory(memory.clone())),
            Vec::new(),
        )),
        _ => Ok((
            "unresolved_conflict".to_string(),
            None,
            candidates
                .into_iter()
                .map(CurrentStateMemoryRef::from_memory)
                .collect(),
        )),
    }
}

fn load_active_memory(conn: &Connection, id: i64, now_epoch: i64) -> Result<Option<Memory>> {
    let sql = format!(
        "SELECT {}
         FROM memories
         WHERE id = ?1
           AND status = 'active'
           AND (expires_at_epoch IS NULL OR expires_at_epoch > ?2)",
        memory::MEMORY_COLS
    );
    conn.query_row(
        &sql,
        rusqlite::params![id, now_epoch],
        memory::map_memory_row_pub,
    )
    .optional()
    .with_context(|| format!("load active current memory id={id}"))
}

fn load_active_state_key_rivals(
    conn: &Connection,
    state_key_id: i64,
    current_memory_id: i64,
    now_epoch: i64,
) -> Result<Vec<CurrentStateMemoryRef>> {
    let sql = format!(
        "SELECT {}
         FROM memories
         WHERE state_key_id = ?1
           AND id <> ?2
           AND status = 'active'
           AND (expires_at_epoch IS NULL OR expires_at_epoch > ?3)
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT ?4",
        memory::MEMORY_COLS
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![state_key_id, current_memory_id, now_epoch, HISTORY_LIMIT],
        memory::map_memory_row_pub,
    )?;
    let memories = crate::db::query::collect_rows(rows)?;
    Ok(memories
        .into_iter()
        .map(CurrentStateMemoryRef::from_memory)
        .collect())
}

fn load_memories_as_of(
    conn: &Connection,
    state_key_id: i64,
    as_of_epoch: i64,
) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {}
         FROM memories
         WHERE state_key_id = ?1
           AND status IN ('active', 'stale')
           AND (valid_from_epoch IS NULL OR valid_from_epoch <= ?2)
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?2)
         ORDER BY COALESCE(valid_from_epoch, created_at_epoch) DESC,
                  updated_at_epoch DESC,
                  id DESC
         LIMIT ?3",
        memory::MEMORY_COLS
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![state_key_id, as_of_epoch, HISTORY_LIMIT],
        memory::map_memory_row_pub,
    )?;
    crate::db::query::collect_rows(rows).context("load current-state memories as-of")
}

fn load_history(conn: &Connection, current_memory_id: i64) -> Result<Vec<CurrentStateMemoryRef>> {
    let sql = format!(
        "SELECT {}, e.edge_type, e.reason, e.evidence_event_ids,
                e.source_candidate_id, e.source_operation_id
         FROM memory_edges e
         JOIN memories m ON m.id = e.from_memory_id
         WHERE e.to_memory_id = ?1
           AND e.edge_type IN ('supersedes', 'merged_into')
         ORDER BY e.created_at_epoch DESC, e.id DESC
         LIMIT ?2",
        prefixed_memory_cols("m")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![current_memory_id, HISTORY_LIMIT], |row| {
        let memory = memory::map_memory_row_pub(row)?;
        ref_from_edge_row(memory, row, 13)
    })?;
    crate::db::query::collect_rows(rows).context("load current-state history")
}

fn load_why(conn: &Connection, current_memory_id: i64) -> Result<Vec<CurrentStateWhy>> {
    let mut stmt = conn.prepare(
        "SELECT edge_type, from_memory_id, to_memory_id, reason, evidence_event_ids,
                source_candidate_id, source_operation_id, created_at_epoch
         FROM memory_edges
         WHERE (to_memory_id = ?1 AND edge_type IN ('supersedes', 'merged_into'))
            OR (edge_type = 'conflicts' AND (from_memory_id = ?1 OR to_memory_id = ?1))
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![current_memory_id, HISTORY_LIMIT], |row| {
        let evidence_json: Option<String> = row.get(4)?;
        Ok(CurrentStateWhy {
            edge_type: row.get(0)?,
            from_memory_id: row.get(1)?,
            to_memory_id: row.get(2)?,
            reason: row.get(3)?,
            evidence_event_ids: parse_evidence_event_ids(evidence_json, 4)?,
            source_candidate_id: row.get(5)?,
            source_operation_id: row.get(6)?,
            created_at_epoch: row.get(7)?,
        })
    })?;
    crate::db::query::collect_rows(rows).context("load current-state why")
}

fn load_facts_for_memory(
    conn: &Connection,
    memory_id: i64,
    as_of_epoch: Option<i64>,
) -> Result<Vec<CurrentStateFact>> {
    let mut conditions = vec!["source_memory_id = ?1".to_string()];
    let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(memory_id)];
    let mut idx = 2;
    if let Some(as_of_epoch) = as_of_epoch {
        conditions.push(format!(
            "(valid_from_epoch IS NULL OR valid_from_epoch <= ?{idx})"
        ));
        conditions.push(format!(
            "(valid_to_epoch IS NULL OR valid_to_epoch > ?{idx})"
        ));
        params_vec.push(Box::new(as_of_epoch));
        idx += 1;
    } else {
        conditions.push("status = 'active'".to_string());
    }

    let sql = format!(
        "SELECT id, subject, predicate, object, valid_from_epoch, valid_to_epoch,
                source_memory_id, source_event_ids, status
         FROM memory_facts
         WHERE {}
         ORDER BY learned_at_epoch DESC, id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    params_vec.push(Box::new(HISTORY_LIMIT));
    let refs = crate::db::to_sql_refs(&params_vec);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        let source_event_json: String = row.get(7)?;
        Ok(CurrentStateFact {
            id: row.get(0)?,
            subject: row.get(1)?,
            predicate: row.get(2)?,
            object: row.get(3)?,
            valid_from_epoch: row.get(4)?,
            valid_to_epoch: row.get(5)?,
            source_memory_id: row.get(6)?,
            source_event_ids: parse_evidence_event_ids(Some(source_event_json), 7)?,
            status: row.get(8)?,
        })
    })?;
    crate::db::query::collect_rows(rows).context("load current-state facts")
}

fn ref_from_edge_row(
    memory: Memory,
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<CurrentStateMemoryRef> {
    let evidence_json: Option<String> = row.get(offset + 2)?;
    Ok(CurrentStateMemoryRef {
        id: memory.id,
        title: memory.title,
        memory_type: memory.memory_type,
        topic_key: memory.topic_key,
        project: memory.project,
        status: memory.status,
        updated_at_epoch: memory.updated_at_epoch,
        relation: row.get(offset)?,
        reason: row.get(offset + 1)?,
        evidence_event_ids: parse_evidence_event_ids(evidence_json, offset + 2)?,
        source_candidate_id: row.get(offset + 3)?,
        source_operation_id: row.get(offset + 4)?,
    })
}

fn parse_evidence_event_ids(raw: Option<String>, column: usize) -> rusqlite::Result<Vec<i64>> {
    match raw {
        Some(json) => serde_json::from_str::<Vec<i64>>(&json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        }),
        None => Ok(Vec::new()),
    }
}

fn prefixed_memory_cols(alias: &str) -> String {
    memory::MEMORY_COLS
        .split(',')
        .map(str::trim)
        .map(|col| format!("{alias}.{col}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl CurrentStateAnswer {
    fn from_memory(memory: Memory) -> Self {
        Self {
            id: memory.id,
            title: memory.title,
            text: memory.text,
            memory_type: memory.memory_type,
            topic_key: memory.topic_key,
            project: memory.project,
            scope: memory.scope,
            status: memory.status,
            updated_at_epoch: memory.updated_at_epoch,
        }
    }
}

impl CurrentStateMemoryRef {
    fn from_memory(memory: Memory) -> Self {
        Self {
            id: memory.id,
            title: memory.title,
            memory_type: memory.memory_type,
            topic_key: memory.topic_key,
            project: memory.project,
            status: memory.status,
            updated_at_epoch: memory.updated_at_epoch,
            relation: None,
            reason: None,
            evidence_event_ids: Vec::new(),
            source_candidate_id: None,
            source_operation_id: None,
        }
    }
}

#[cfg(test)]
mod tests;
