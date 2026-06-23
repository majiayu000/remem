use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use rusqlite::{types::ToSql, Connection, OptionalExtension};

use crate::memory::{self, Memory, MemoryStalenessLabel};
use types::CurrentStateMemoryRefParts;

mod types;

pub use types::{
    CurrentStateAnswer, CurrentStateFact, CurrentStateKeySummary, CurrentStateMemoryRef,
    CurrentStateRequest, CurrentStateResult, CurrentStateWhy,
};

const HISTORY_LIMIT: i64 = 10;

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
            .map(|current| load_history(conn, state.id, current.id, req.as_of_epoch))
            .transpose()?
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let why = current
        .as_ref()
        .map(|current| load_why(conn, state.id, current.id, req.as_of_epoch))
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
            idx += 1;
        }
        (Some(_), None) | (None, Some(_)) => {
            bail!("owner_scope and owner_key must be provided together");
        }
        (None, None) => {
            let project = project_filter(req)?;
            conditions.push(format!(
                "((owner_scope = 'repo' AND owner_key = ?{idx})
                   OR (owner_scope = 'user' AND owner_key = 'user:default'))"
            ));
            params_vec.push(Box::new(project));
            idx += 1;
        }
    }
    if let Some(as_of_epoch) = req.as_of_epoch {
        conditions.push(format!("created_at_epoch <= ?{idx}"));
        params_vec.push(Box::new(as_of_epoch));
    }

    let sql = format!(
        "SELECT id, owner_scope, owner_key, memory_type, state_key, state_label,
                state_status,
                CASE
                    WHEN current_memory_id IS NULL THEN NULL
                    WHEN EXISTS (
                        SELECT 1
                        FROM memories cm
                        WHERE cm.id = memory_state_keys.current_memory_id
                          AND {policy_filter}
                    ) THEN current_memory_id
                    ELSE NULL
                END
         FROM memory_state_keys
         WHERE {}
         ORDER BY
             CASE owner_scope WHEN 'repo' THEN 0 WHEN 'user' THEN 1 ELSE 2 END,
             updated_at_epoch DESC,
             id DESC
         LIMIT 3",
        conditions.join(" AND "),
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("cm"),
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

fn project_filter(req: &CurrentStateRequest) -> Result<String> {
    if let Some(project) = req
        .project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(project.to_string());
    }
    let cwd =
        std::env::current_dir().context("resolve current project for current-state lookup")?;
    Ok(crate::db::project_from_cwd(cwd.to_string_lossy().as_ref()))
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
    let current_memory = state
        .current_memory_id
        .map(|id| load_active_memory(conn, id, now_epoch))
        .transpose()?
        .flatten();
    let conflict_parts = match current_memory.as_ref() {
        Some(current) => load_active_state_key_rivals(conn, state.id, current.id, now_epoch)?,
        None => load_active_state_key_rivals(conn, state.id, -1, now_epoch)?,
    };
    let mut memories = Vec::new();
    if let Some(memory) = &current_memory {
        memories.push(memory.clone());
    }
    memories.extend(conflict_parts.iter().map(|parts| parts.memory.clone()));
    let staleness_labels = staleness_labels_for_memories(conn, &memories, now_epoch)?;
    let current = current_memory.map(|memory| {
        let staleness = staleness_label_for_memory(&staleness_labels, &memory, now_epoch);
        CurrentStateAnswer::from_memory(memory, staleness)
    });
    let conflicts = conflict_parts
        .into_iter()
        .map(|parts| {
            let staleness = staleness_label_for_memory(&staleness_labels, &parts.memory, now_epoch);
            CurrentStateMemoryRef::from_parts(parts, staleness)
        })
        .collect::<Vec<_>>();

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
    let now_epoch = chrono::Utc::now().timestamp();
    let candidates = load_memories_as_of(conn, state_key_id, as_of_epoch)?;
    match candidates.as_slice() {
        [] => Ok(("no_current".to_string(), None, Vec::new())),
        [memory] => {
            let staleness_labels =
                staleness_labels_for_memories(conn, std::slice::from_ref(memory), now_epoch)?;
            let staleness = staleness_label_for_memory(&staleness_labels, memory, now_epoch);
            Ok((
                "current".to_string(),
                Some(CurrentStateAnswer::from_memory(memory.clone(), staleness)),
                Vec::new(),
            ))
        }
        _ => {
            let staleness_labels = staleness_labels_for_memories(conn, &candidates, now_epoch)?;
            Ok((
                "unresolved_conflict".to_string(),
                None,
                candidates
                    .into_iter()
                    .map(|memory| {
                        let staleness =
                            staleness_label_for_memory(&staleness_labels, &memory, now_epoch);
                        CurrentStateMemoryRef::from_memory(memory, staleness)
                    })
                    .collect(),
            ))
        }
    }
}

fn load_active_memory(conn: &Connection, id: i64, now_epoch: i64) -> Result<Option<Memory>> {
    let sql = format!(
        "SELECT {}
         FROM memories
         WHERE id = ?1
           AND status = 'active'
           AND COALESCE(valid_from_epoch, created_at_epoch) <= ?2
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?2)
           AND (expires_at_epoch IS NULL OR expires_at_epoch > ?2)
           AND {policy_filter}",
        memory::MEMORY_COLS,
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("memories"),
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
) -> Result<Vec<CurrentStateMemoryRefParts>> {
    let sql = format!(
        "SELECT {}, e.edge_type, e.reason, e.evidence_event_ids,
                e.source_candidate_id, e.source_operation_id
         FROM memories m
         LEFT JOIN memory_edges e ON e.id = (
             SELECT ce.id
             FROM memory_edges ce
             WHERE ce.edge_type = 'conflicts'
               AND ce.state_key_id = ?1
               AND ((ce.from_memory_id = m.id AND ce.to_memory_id = ?2)
                    OR (ce.from_memory_id = ?2 AND ce.to_memory_id = m.id))
             ORDER BY ce.created_at_epoch DESC, ce.id DESC
             LIMIT 1
         )
         WHERE m.state_key_id = ?1
           AND m.id <> ?2
           AND m.status = 'active'
           AND COALESCE(m.valid_from_epoch, m.created_at_epoch) <= ?3
           AND (m.valid_to_epoch IS NULL OR m.valid_to_epoch > ?3)
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?3)
           AND {policy_filter}
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?4",
        prefixed_memory_cols("m"),
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("m"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![state_key_id, current_memory_id, now_epoch, HISTORY_LIMIT],
        |row| {
            let memory = memory::map_memory_row_pub(row)?;
            ref_parts_from_edge_row(memory, row, 13)
        },
    )?;
    crate::db::query::collect_rows(rows).context("load active current-state rivals")
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
           AND (status = 'active'
                OR (status = 'stale'
                    AND (valid_to_epoch IS NOT NULL OR updated_at_epoch > ?2))
                OR (status = 'archived' AND valid_to_epoch IS NOT NULL))
           AND COALESCE(valid_from_epoch, created_at_epoch) <= ?2
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?2)
           AND (status <> 'active' OR updated_at_epoch <= ?2)
           AND (expires_at_epoch IS NULL OR expires_at_epoch > ?2)
           AND {policy_filter}
         ORDER BY COALESCE(valid_from_epoch, created_at_epoch) DESC,
                  updated_at_epoch DESC,
                  id DESC
         LIMIT ?3",
        memory::MEMORY_COLS,
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("memories"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![state_key_id, as_of_epoch, HISTORY_LIMIT],
        memory::map_memory_row_pub,
    )?;
    crate::db::query::collect_rows(rows).context("load current-state memories as-of")
}

fn load_history(
    conn: &Connection,
    state_key_id: i64,
    current_memory_id: i64,
    as_of_epoch: Option<i64>,
) -> Result<Vec<CurrentStateMemoryRef>> {
    let mut params_vec: Vec<Box<dyn ToSql>> = vec![
        Box::new(current_memory_id),
        Box::new(state_key_id),
        Box::new(HISTORY_LIMIT),
    ];
    let mut as_of_filter = String::new();
    if let Some(as_of_epoch) = as_of_epoch {
        as_of_filter.push_str(
            " AND e.created_at_epoch <= ?4
           AND COALESCE(m.valid_from_epoch, m.created_at_epoch) <= ?4
           AND m.updated_at_epoch <= ?4",
        );
        params_vec.push(Box::new(as_of_epoch));
    }
    let sql = format!(
        "SELECT {}, e.edge_type, e.reason, e.evidence_event_ids,
                e.source_candidate_id, e.source_operation_id
         FROM memory_edges e
         JOIN memories m ON m.id = e.from_memory_id
         WHERE e.to_memory_id = ?1
           AND e.state_key_id = ?2
           AND e.edge_type IN ('supersedes', 'merged_into')
           AND {policy_filter}
           {as_of_filter}
         ORDER BY e.created_at_epoch DESC, e.id DESC
         LIMIT ?3",
        prefixed_memory_cols("m"),
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("m"),
    );
    let refs = crate::db::to_sql_refs(&params_vec);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        let memory = memory::map_memory_row_pub(row)?;
        ref_parts_from_edge_row(memory, row, 13)
    })?;
    let parts = crate::db::query::collect_rows(rows).context("load current-state history")?;
    let now_epoch = chrono::Utc::now().timestamp();
    let memories = parts
        .iter()
        .map(|parts| parts.memory.clone())
        .collect::<Vec<_>>();
    let staleness_labels = staleness_labels_for_memories(conn, &memories, now_epoch)?;
    Ok(parts
        .into_iter()
        .map(|parts| {
            let staleness = staleness_label_for_memory(&staleness_labels, &parts.memory, now_epoch);
            CurrentStateMemoryRef::from_parts(parts, staleness)
        })
        .collect())
}

fn load_why(
    conn: &Connection,
    state_key_id: i64,
    current_memory_id: i64,
    as_of_epoch: Option<i64>,
) -> Result<Vec<CurrentStateWhy>> {
    let mut params_vec: Vec<Box<dyn ToSql>> = vec![
        Box::new(current_memory_id),
        Box::new(state_key_id),
        Box::new(HISTORY_LIMIT),
    ];
    let mut as_of_filter = String::new();
    if let Some(as_of_epoch) = as_of_epoch {
        as_of_filter.push_str(" AND created_at_epoch <= ?4");
        params_vec.push(Box::new(as_of_epoch));
    }
    let sql = format!(
        "SELECT edge_type, from_memory_id, to_memory_id, reason, evidence_event_ids,
                source_candidate_id, source_operation_id, created_at_epoch
         FROM memory_edges
         WHERE ((to_memory_id = ?1
                  AND edge_type IN ('supersedes', 'merged_into')
                  AND state_key_id = ?2)
             OR (to_memory_id = ?1
                 AND edge_type = 'derived_from'
                 AND (state_key_id = ?2 OR state_key_id IS NULL))
             OR (edge_type = 'conflicts'
                 AND (from_memory_id = ?1 OR to_memory_id = ?1)
                 AND state_key_id = ?2))
           AND (from_memory_id IS NULL OR EXISTS (
                 SELECT 1 FROM memories fm
                 WHERE fm.id = from_memory_id
                   AND {from_policy_filter}
           ))
           AND (to_memory_id IS NULL OR EXISTS (
                 SELECT 1 FROM memories tm
                 WHERE tm.id = to_memory_id
                   AND {to_policy_filter}
           ))
           {as_of_filter}
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT ?3",
        from_policy_filter = crate::memory::suppression::memory_policy_filter_sql("fm"),
        to_policy_filter = crate::memory::suppression::memory_policy_filter_sql("tm"),
    );
    let refs = crate::db::to_sql_refs(&params_vec);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
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
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    let effective_epoch = as_of_epoch.unwrap_or_else(|| chrono::Utc::now().timestamp());
    conditions.push(format!(
        "(valid_from_epoch IS NULL OR valid_from_epoch <= ?{idx})"
    ));
    conditions.push(crate::memory::facts::as_of_validity_filter_sql(
        "",
        idx,
        has_invalidated_at_epoch,
    ));
    params_vec.push(Box::new(effective_epoch));
    idx += 1;
    if let Some(as_of_epoch) = as_of_epoch {
        conditions.push(format!("learned_at_epoch <= ?{idx}"));
        if has_invalidated_at_epoch {
            conditions.push(format!(
                "(invalidated_at_epoch IS NULL OR invalidated_at_epoch > ?{idx})"
            ));
        }
        params_vec.push(Box::new(as_of_epoch));
        idx += 1;
    }
    if as_of_epoch.is_none() {
        conditions.push(crate::memory::facts::current_fact_filter_sql(
            "",
            has_invalidated_at_epoch,
        ));
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

fn ref_parts_from_edge_row(
    memory: Memory,
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<CurrentStateMemoryRefParts> {
    let evidence_json: Option<String> = row.get(offset + 2)?;
    Ok(CurrentStateMemoryRefParts {
        memory,
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

fn staleness_labels_for_memories(
    conn: &Connection,
    memories: &[Memory],
    now_epoch: i64,
) -> Result<HashMap<i64, MemoryStalenessLabel>> {
    memory::memory_staleness_labels_for_memories_lossy(conn, memories, now_epoch, |_id, _err| {})
        .context("load current-state staleness labels")
}

fn staleness_label_for_memory(
    labels: &HashMap<i64, MemoryStalenessLabel>,
    memory: &Memory,
    now_epoch: i64,
) -> MemoryStalenessLabel {
    labels.get(&memory.id).cloned().unwrap_or_else(|| {
        memory::memory_staleness_error_label(
            memory,
            now_epoch,
            format!("missing staleness label for memory id={}", memory.id),
        )
    })
}

#[cfg(test)]
mod tests;
