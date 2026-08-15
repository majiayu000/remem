use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::{load_memory_relations, memory_ref, trust_class_evidence, MemoryRow};
use crate::truth::lifecycle::memory_lifecycle;
use crate::truth::types::{
    ClaimSource, ClaimView, EvidenceKind, EvidenceTrust, EvidenceView, RelationView, TruthQuery,
    Visibility,
};

pub(super) fn load(
    conn: &Connection,
    query: &TruthQuery,
    reference_epoch: i64,
    relevant_memory_ids: &[i64],
) -> Result<(Vec<ClaimView>, Vec<RelationView>)> {
    if relevant_memory_ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let projects = crate::project_alias::project_filter_values(conn, &query.project)?;
    let owner_filter = "((memories.owner_scope = 'repo'
              AND memories.owner_key IN (SELECT value FROM json_each(?2)))
          OR (memories.owner_scope = 'repo'
              AND memories.target_project IN (SELECT value FROM json_each(?2)))
          OR (memories.owner_scope IS NULL
              AND memories.project IN (SELECT value FROM json_each(?2))
              AND COALESCE(memories.scope, 'project') != 'global'))";
    let sql = format!(
        "WITH relevant AS (
             SELECT seed.memory_type, seed.topic_key, seed_state.state_key
             FROM memories seed
             LEFT JOIN memory_state_keys seed_state ON seed_state.id = seed.state_key_id
             WHERE seed.id IN (SELECT value FROM json_each(?))
         )
         SELECT memories.id, memories.topic_key, memories.title, memories.content,
                memories.project, memories.branch, memories.status,
                memories.valid_from_epoch, memories.valid_to_epoch, memories.expires_at_epoch,
                memories.created_at_epoch, memories.updated_at_epoch,
                memories.evidence_event_ids, memories.source_trust_class,
                memories.memory_type, memories.state_key_id,
                COALESCE(memories.owner_scope, 'repo'),
                COALESCE(memories.owner_key, memories.target_project, memories.project),
                state_key.state_key, memories.owner_scope IS NOT NULL
         FROM memories
         LEFT JOIN memory_state_keys state_key ON state_key.id = memories.state_key_id
         WHERE {owner_filter}
           AND memories.memory_type IN ('bugfix', 'architecture', 'decision', 'discovery')
           AND (?3 IS NULL OR memories.branch IS NULL OR memories.branch = ?3)
           AND EXISTS (
               SELECT 1 FROM relevant
               WHERE relevant.memory_type = memories.memory_type
                 AND ((relevant.state_key IS NOT NULL
                       AND relevant.state_key = state_key.state_key)
                      OR (relevant.state_key IS NULL AND state_key.state_key IS NULL
                          AND relevant.topic_key IS memories.topic_key)))
         ORDER BY memories.updated_at_epoch ASC, memories.id ASC"
    );
    let ids_json = serde_json::to_string(relevant_memory_ids)?;
    let projects_json = serde_json::to_string(&projects)?;
    let mut stmt = conn
        .prepare(&sql)
        .context("prepare live CurrentTruth query")?;
    let rows = stmt
        .query_map(
            rusqlite::params![ids_json, projects_json, query.branch],
            map_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("read live CurrentTruth rows")?;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let visibility = crate::truth::admit_many_for_current_context(conn, &ids, reference_epoch)?;
    let evidence = load_evidence(conn, &rows)?;
    let canonical_scope =
        crate::project_alias::resolve_project_identity(conn, &query.project)?.canonical_path;
    let canonical_owners = super::canonical_owner_keys(conn, &rows)?;
    let mut claims = Vec::with_capacity(rows.len());
    for row in rows {
        let mut claim = claim_view(row, &canonical_scope, &canonical_owners, &evidence);
        if !visibility
            .get(&claim_id(&claim))
            .context("missing live CurrentTruth visibility row")?
            .current_context_eligible
        {
            claim.lifecycle.visibility = Visibility::Suppressed;
        }
        claims.push(claim);
    }
    let admitted_ids = ids
        .into_iter()
        .filter(|id| {
            visibility
                .get(id)
                .is_some_and(|row| row.current_context_eligible)
        })
        .collect::<Vec<_>>();
    let relations = load_memory_relations(conn, &admitted_ids, true)?;
    Ok((claims, relations))
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRow> {
    Ok(MemoryRow {
        id: row.get(0)?,
        topic_key: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        _project: row.get(4)?,
        branch: row.get(5)?,
        status: row.get(6)?,
        valid_from_epoch: row.get(7)?,
        valid_to_epoch: row.get(8)?,
        expires_at_epoch: row.get(9)?,
        created_at_epoch: row.get(10)?,
        updated_at_epoch: row.get(11)?,
        evidence_event_ids: row.get(12)?,
        source_trust_class: row.get(13)?,
        memory_type: row.get(14)?,
        owner_scope: row.get(16)?,
        owner_key: row.get(17)?,
        state_key: row.get(18)?,
        owner_explicit: row.get(19)?,
    })
}

fn load_evidence(conn: &Connection, rows: &[MemoryRow]) -> Result<HashMap<i64, EvidenceView>> {
    let ids = rows
        .iter()
        .filter_map(|row| row.evidence_event_ids.as_deref())
        .filter_map(|raw| serde_json::from_str::<Vec<i64>>(raw).ok())
        .flatten()
        .collect::<BTreeSet<_>>();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let json = serde_json::to_string(&ids)?;
    let mut stmt = conn.prepare(
        "SELECT id, event_type, role, tool_name, created_at_epoch
         FROM captured_events WHERE id IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = stmt.query_map([json], |row| {
        let id: i64 = row.get(0)?;
        let event_type: String = row.get(1)?;
        let role: Option<String> = row.get(2)?;
        let tool: Option<String> = row.get(3)?;
        let trust = match (role.as_deref(), tool.as_deref()) {
            (Some("user"), _) | (Some("tool"), _) | (_, Some(_)) => EvidenceTrust::Verified,
            _ => EvidenceTrust::ModelGenerated,
        };
        Ok((
            id,
            EvidenceView {
                evidence_ref: format!("captured_event:{id}"),
                kind: EvidenceKind::CapturedEvent,
                source_ref: role
                    .map_or_else(|| event_type.clone(), |role| format!("{event_type}/{role}")),
                observed_at_epoch: Some(row.get(4)?),
                trust,
            },
        ))
    })?;
    Ok(crate::db::query::collect_rows(rows)?.into_iter().collect())
}

fn claim_view(
    row: MemoryRow,
    canonical_scope: &str,
    canonical_owners: &HashMap<String, String>,
    evidence_by_id: &HashMap<i64, EvidenceView>,
) -> ClaimView {
    let mut evidence = row
        .evidence_event_ids
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<i64>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| evidence_by_id.get(&id).cloned())
        .collect::<Vec<_>>();
    if let Some(extra) = trust_class_evidence(row.id, row.source_trust_class.as_deref()) {
        evidence.push(extra);
    }
    let effective_valid_to = match (row.valid_to_epoch, row.expires_at_epoch) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    ClaimView {
        canonical_ref: memory_ref(row.id),
        source: ClaimSource::Memory,
        subject_key: super::memory_subject_key(&row, canonical_owners),
        statement: format!("{}: {}", row.title, row.content),
        scope: canonical_scope.to_string(),
        branch: row.branch,
        lifecycle: memory_lifecycle(&row.status),
        valid_from_epoch: row.valid_from_epoch,
        valid_to_epoch: effective_valid_to,
        created_at_epoch: row.created_at_epoch,
        updated_at_epoch: row.updated_at_epoch,
        evidence,
    }
}

fn claim_id(claim: &ClaimView) -> i64 {
    claim
        .canonical_ref
        .strip_prefix("memory:")
        .and_then(|id| id.parse().ok())
        .unwrap_or(-1)
}
