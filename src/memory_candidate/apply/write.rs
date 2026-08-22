use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::poisoning::SourceTrustClass;

use super::{CandidateRoute, ParsedMemoryCandidate};

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_routed_memory(
    conn: &Connection,
    session_id: Option<&str>,
    source_project: &str,
    memory_project: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    title: &str,
    evidence_json: &str,
    scope: &str,
    state_key: Option<&crate::memory::state_key::StateKeyDecision>,
    reference_time_epoch: i64,
    source_trust: SourceTrustClass,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let (expires_at_epoch, valid_from_epoch) = crate::memory::lifecycle::ttl_metadata(
        &candidate.memory_type,
        Some(&candidate.topic_key),
        &candidate.text,
        now,
    );
    let search_context = crate::memory::search_context::build_search_context(
        &candidate.memory_type,
        Some(&candidate.topic_key),
        &candidate.text,
        None,
    );
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          evidence_event_ids, source_candidate_id, confidence,
          source_project, target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, routing_reason, context_class, expires_at_epoch,
          valid_from_epoch, source_trust_class)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7,
                 ?8, ?8, ?9, 'active', NULL, ?10,
                 ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
        params![
            session_id,
            memory_project,
            candidate.topic_key,
            title,
            candidate.text,
            candidate.memory_type,
            search_context,
            now,
            reference_time_epoch,
            scope,
            evidence_json,
            candidate_id,
            candidate.confidence,
            source_project,
            route.target_project.as_deref(),
            route.owner_scope,
            route.owner_key,
            route.topic_domain.as_deref(),
            route.routing_confidence,
            route.routing_reason,
            route.context_class,
            expires_at_epoch,
            valid_from_epoch,
            source_trust.as_str()
        ],
    )?;
    let memory_id = conn.last_insert_rowid();
    if let Some(state_key) = state_key {
        crate::memory::state_key::attach_current_memory(
            conn,
            memory_id,
            &route.owner_scope,
            &route.owner_key,
            &candidate.memory_type,
            state_key,
            now,
        )?;
    }
    if candidate.memory_type == "lesson" {
        insert_lesson_metadata(conn, memory_id, candidate, evidence_json, now)?;
    }
    refresh_memory_entities(conn, memory_id, title, &candidate.text)?;
    crate::retrieval::vector::upsert_memory_embedding_for_row(conn, memory_id)?;
    Ok(memory_id)
}

pub(super) fn insert_candidate_event_time_fact(
    conn: &Connection,
    memory_project: &str,
    memory_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_event_ids: &[i64],
    valid_from_epoch: i64,
) -> Result<i64> {
    crate::memory::facts::insert_temporal_fact_in_current_tx(
        conn,
        &crate::memory::facts::TemporalFactInput {
            project: memory_project,
            subject: &candidate.topic_key,
            predicate: crate::memory::facts::FactPredicate::AffectsProject,
            object: memory_project,
            valid_from_epoch: Some(valid_from_epoch),
            valid_to_epoch: None,
            learned_at_epoch: None,
            source_memory_id: Some(memory_id),
            source_observation_id: None,
            source_event_ids: evidence_event_ids,
            confidence: candidate.confidence,
            supersedes_fact_id: None,
        },
        chrono::Utc::now().timestamp(),
    )
}

pub(super) fn evidence_valid_from_epoch(
    conn: &Connection,
    evidence_event_ids: &[i64],
) -> Result<i64> {
    if evidence_event_ids.is_empty() {
        bail!("candidate promotion requires evidence_event_ids for temporal fact");
    }
    let mut earliest = None;
    for event_id in evidence_event_ids {
        let epoch: i64 = conn
            .query_row(
                "SELECT COALESCE(reference_time_epoch, created_at_epoch)
                 FROM captured_events
                 WHERE id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?
            .with_context(|| {
                format!("candidate evidence event id={event_id} missing for temporal fact")
            })?;
        earliest = Some(earliest.map_or(epoch, |current: i64| current.min(epoch)));
    }
    earliest.context("candidate promotion requires evidence_event_ids for temporal fact")
}

pub(super) fn soft_supersede_routed(
    conn: &Connection,
    memory_ids: &[i64],
    replacement_id: Option<i64>,
) -> Result<usize> {
    let mut seen = std::collections::HashSet::with_capacity(memory_ids.len());
    let targets = memory_ids
        .iter()
        .copied()
        .filter(|id| Some(*id) != replacement_id && seen.insert(*id))
        .collect::<Vec<_>>();
    let mut changed = 0usize;
    for id in targets {
        let updated = conn.execute(
            "UPDATE memories
             SET status = 'stale',
                 valid_to_epoch = COALESCE(valid_to_epoch, ?2)
             WHERE id = ?1",
            params![id, chrono::Utc::now().timestamp()],
        )?;
        if updated != 1 {
            bail!("failed to mark superseded memory stale: id={id}");
        }
        changed += updated;
    }
    Ok(changed)
}

fn insert_lesson_metadata(
    conn: &Connection,
    memory_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    now: i64,
) -> Result<()> {
    let outcome_kind = candidate.outcome.as_deref().unwrap_or("unknown");
    let success = i64::from(outcome_kind == "success");
    let failure = i64::from(outcome_kind == "failure");
    conn.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
          success_count, failure_count, recovery_count, correction_count, revert_count)
         VALUES (?1, ?2, 1, ?3, ?4, NULL, ?5, ?6, ?7, 0, 0, 0)",
        params![
            memory_id,
            candidate.confidence,
            evidence_json,
            now,
            outcome_kind,
            success,
            failure
        ],
    )?;
    Ok(())
}

fn refresh_memory_entities(conn: &Connection, id: i64, title: &str, content: &str) -> Result<()> {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    crate::retrieval::entity::refresh_memory_entities(conn, id, &entities)
        .with_context(|| format!("entity refresh failed for memory id={id}"))
}
