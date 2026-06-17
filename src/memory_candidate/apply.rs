use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{candidate_title, CandidateRoute, ParsedMemoryCandidate};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{
    insert_operation_log, same_memory_text, with_operation_savepoint, MemoryOperationInput,
    MemoryOperationPlan,
};
use crate::memory::preference::consolidation::{
    load_active_preference_content, PreferenceConsolidationKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CandidateApplyOutcome {
    pub memory_id: Option<i64>,
    pub promoted: bool,
    pub noop: bool,
    pub superseded: usize,
}

impl CandidateApplyOutcome {
    pub(super) fn review_status_for<'a>(&self, promoted_status: &'a str) -> &'a str {
        if self.noop {
            "noop"
        } else {
            promoted_status
        }
    }
}

pub(super) fn promote_candidate_to_memory_with_route(
    conn: &Connection,
    session_id: Option<&str>,
    source_project: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    route: &CandidateRoute,
) -> Result<CandidateApplyOutcome> {
    let title = candidate_title(candidate);
    let memory_project = route.memory_project(source_project);
    let memory_scope = route.memory_scope();

    with_operation_savepoint(conn, || {
        let now = chrono::Utc::now().timestamp();
        let candidate_has_ttl = crate::memory::lifecycle::default_ttl_seconds(
            &candidate.memory_type,
            Some(&candidate.topic_key),
            &candidate.text,
        )
        .is_some();
        let state_key = crate::memory::state_key::derive_state_key(
            &candidate.memory_type,
            Some(&candidate.topic_key),
            &title,
            &candidate.text,
        );
        let state_key_value = state_key
            .as_ref()
            .map(|decision| decision.state_key.clone());
        let operation_input = MemoryOperationInput {
            source: "memory_candidate".to_string(),
            actor: "memory_candidate".to_string(),
            source_project: source_project.to_string(),
            owner_scope: route.owner_scope.clone(),
            owner_key: route.owner_key.clone(),
            memory_type: candidate.memory_type.clone(),
            topic_key: Some(candidate.topic_key.clone()),
            state_key: state_key_value.clone(),
            source_candidate_id: Some(candidate_id),
            confidence: Some(candidate.confidence),
        };
        let mut active = find_active_same_state_or_topic(
            conn,
            candidate,
            route,
            state_key.as_ref(),
            now,
            candidate_has_ttl,
        )?;
        let mut generic_preference_reason = None;
        let mut conflicting_ids = Vec::new();
        if candidate.memory_type == "preference" && active.is_empty() {
            if let Some(preference_match) =
                crate::memory::preference::consolidation::find_preference_consolidation(
                    conn,
                    &route.owner_scope,
                    &route.owner_key,
                    memory_scope,
                    None,
                    &candidate.text,
                    now,
                )?
            {
                generic_preference_reason = Some(preference_match.reason.clone());
                match preference_match.kind {
                    PreferenceConsolidationKind::SamePreference
                    | PreferenceConsolidationKind::Refinement => {
                        active.push(ActiveTopicMemory {
                            id: preference_match.memory_id,
                            content: load_active_preference_content(
                                conn,
                                preference_match.memory_id,
                            )?,
                            is_current: true,
                        });
                    }
                    PreferenceConsolidationKind::Contradiction => {
                        conflicting_ids.push(preference_match.memory_id);
                    }
                }
            }
        }
        if let Some(existing) = active
            .iter()
            .filter(|row| row.is_current)
            .find(|row| same_memory_text(&row.content, &candidate.text))
        {
            let plan = MemoryOperationPlan::new(
                MemoryLifecycleOp::Noop,
                state_key_value,
                "candidate already represented by active memory",
            )
            .with_target_memory_id(Some(existing.id))
            .with_noop_reason("already represented by active memory");
            insert_operation_log(conn, &operation_input, &plan, Some(existing.id))?;
            return Ok(CandidateApplyOutcome {
                memory_id: Some(existing.id),
                promoted: false,
                noop: true,
                superseded: 0,
            });
        }

        let superseded_ids = active.iter().map(|row| row.id).collect::<Vec<_>>();
        let op = if !conflicting_ids.is_empty() {
            MemoryLifecycleOp::Conflict
        } else if superseded_ids.is_empty() {
            MemoryLifecycleOp::Add
        } else {
            MemoryLifecycleOp::Update
        };
        let reason = if let Some(reason) = generic_preference_reason {
            reason
        } else if !conflicting_ids.is_empty() {
            "candidate conflicts with active preference memories".to_string()
        } else if superseded_ids.is_empty() {
            "candidate creates new current memory".to_string()
        } else {
            "candidate replaces active state/topic memories".to_string()
        };
        let mut plan = MemoryOperationPlan::new(op, state_key_value, reason)
            .with_superseded_ids(superseded_ids.clone())
            .with_conflicting_ids(conflicting_ids.clone());

        let evidence_event_ids: Vec<i64> = serde_json::from_str(evidence_json)?;
        let reference_time_epoch = evidence_valid_from_epoch(conn, &evidence_event_ids)?;
        let memory_id = insert_routed_memory(
            conn,
            session_id,
            source_project,
            &memory_project,
            candidate_id,
            candidate,
            route,
            &title,
            evidence_json,
            memory_scope,
            state_key.as_ref(),
            reference_time_epoch,
        )?;
        plan.target_memory_id = Some(memory_id);
        let superseded = soft_supersede_routed(conn, &superseded_ids, Some(memory_id))?;
        let operation_id = insert_operation_log(conn, &operation_input, &plan, Some(memory_id))?;
        crate::memory::edge::insert_memory_edge(
            conn,
            &crate::memory::edge::MemoryEdgeInput {
                edge_type: crate::memory::edge::MemoryEdgeType::DerivedFrom,
                from_memory_id: None,
                to_memory_id: Some(memory_id),
                state_key_id: None,
                source_candidate_id: Some(candidate_id),
                evidence_event_ids: &evidence_event_ids,
                source_operation_id: Some(operation_id),
                confidence: Some(candidate.confidence),
                reason: Some("candidate promoted from observation evidence"),
            },
        )?;
        crate::memory::edge::insert_supersedes_edges(
            conn,
            &superseded_ids,
            memory_id,
            crate::memory::edge::MemoryEdgeWriteContext {
                source_candidate_id: Some(candidate_id),
                evidence_event_ids: &evidence_event_ids,
                source_operation_id: Some(operation_id),
                confidence: Some(candidate.confidence),
                reason: Some(plan.reason.as_str()),
                ..Default::default()
            },
        )?;
        crate::memory::edge::insert_conflicts_edges(
            conn,
            &conflicting_ids,
            memory_id,
            crate::memory::edge::MemoryEdgeWriteContext {
                source_candidate_id: Some(candidate_id),
                evidence_event_ids: &evidence_event_ids,
                source_operation_id: Some(operation_id),
                confidence: Some(candidate.confidence),
                reason: Some(plan.reason.as_str()),
                ..Default::default()
            },
        )?;
        insert_candidate_event_time_fact(
            conn,
            &memory_project,
            memory_id,
            candidate,
            &evidence_event_ids,
            reference_time_epoch,
        )
        .with_context(|| {
            format!("failed to write temporal fact for promoted candidate id={candidate_id}")
        })?;
        Ok(CandidateApplyOutcome {
            memory_id: Some(memory_id),
            promoted: true,
            noop: false,
            superseded,
        })
    })
}

pub(super) fn update_candidate_after_lifecycle(
    conn: &Connection,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    review_status: &str,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let (expires_at_epoch, valid_from_epoch) = crate::memory::lifecycle::ttl_metadata(
        &candidate.memory_type,
        Some(&candidate.topic_key),
        &candidate.text,
        now,
    );
    let title = candidate_title(candidate);
    let state_key = crate::memory::state_key::derive_state_key(
        &candidate.memory_type,
        Some(&candidate.topic_key),
        &title,
        &candidate.text,
    );
    conn.execute(
        "UPDATE memory_candidates
         SET scope = ?1,
             memory_type = ?2,
             topic_key = ?3,
             text = ?4,
             review_status = ?5,
             updated_at_epoch = ?6,
             target_project = ?7,
             owner_scope = ?8,
             owner_key = ?9,
             topic_domain = ?10,
             routing_confidence = ?11,
             routing_reason = ?12,
             context_class = ?13,
             expires_at_epoch = ?14,
             valid_from_epoch = ?15,
             state_key = ?16,
             state_key_confidence = ?17,
             state_key_reason = ?18
         WHERE id = ?19",
        params![
            candidate.scope,
            candidate.memory_type,
            candidate.topic_key,
            candidate.text,
            review_status,
            now,
            route.target_project.as_deref(),
            route.owner_scope,
            route.owner_key,
            route.topic_domain.as_deref(),
            route.routing_confidence,
            route.routing_reason,
            route.context_class,
            expires_at_epoch,
            valid_from_epoch,
            state_key
                .as_ref()
                .map(|decision| decision.state_key.as_str()),
            state_key.as_ref().map(|decision| decision.confidence),
            state_key.as_ref().map(|decision| decision.reason.as_str()),
            candidate_id
        ],
    )?;
    Ok(())
}

#[derive(Debug)]
struct ActiveTopicMemory {
    id: i64,
    content: String,
    is_current: bool,
}

fn find_active_same_topic(
    conn: &Connection,
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    now_epoch: i64,
    candidate_has_ttl: bool,
) -> Result<Vec<ActiveTopicMemory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content,
                CASE
                    WHEN ?5 = 1 THEN
                        CASE
                            WHEN expires_at_epoch IS NOT NULL AND expires_at_epoch > ?6 THEN 1
                            ELSE 0
                        END
                    WHEN expires_at_epoch IS NULL OR expires_at_epoch > ?6 THEN 1
                    ELSE 0
                END AS is_current
         FROM memories
         WHERE status = 'active'
           AND memory_type = ?1
           AND topic_key = ?2
           AND COALESCE(
                owner_scope,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
           ) = ?3
           AND COALESCE(
                owner_key,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
           ) = ?4
         ORDER BY updated_at_epoch DESC, id DESC",
    )?;
    let rows = stmt.query_map(
        params![
            candidate.memory_type,
            candidate.topic_key,
            route.owner_scope,
            route.owner_key,
            if candidate_has_ttl { 1_i64 } else { 0_i64 },
            now_epoch
        ],
        |row| {
            Ok(ActiveTopicMemory {
                id: row.get(0)?,
                content: row.get(1)?,
                is_current: row.get::<_, i64>(2)? == 1,
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
}

fn find_active_same_state_or_topic(
    conn: &Connection,
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    state_key: Option<&crate::memory::state_key::StateKeyDecision>,
    now_epoch: i64,
    candidate_has_ttl: bool,
) -> Result<Vec<ActiveTopicMemory>> {
    if let Some(state_key) = state_key {
        let ids = crate::memory::state_key::active_memory_ids(
            conn,
            &route.owner_scope,
            &route.owner_key,
            &candidate.memory_type,
            &state_key.state_key,
            now_epoch,
            candidate_has_ttl,
        )?;
        if !ids.is_empty() {
            let mut memories = Vec::with_capacity(ids.len());
            for id in ids {
                let content =
                    conn.query_row("SELECT content FROM memories WHERE id = ?1", [id], |row| {
                        row.get(0)
                    })?;
                memories.push(ActiveTopicMemory {
                    id,
                    content,
                    is_current: true,
                });
            }
            return Ok(memories);
        }
    }
    find_active_same_topic(conn, candidate, route, now_epoch, candidate_has_ttl)
}

#[allow(clippy::too_many_arguments)]
fn insert_routed_memory(
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
          valid_from_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7,
                 ?8, ?8, ?9, 'active', NULL, ?10,
                 ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
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
            valid_from_epoch
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

fn insert_candidate_event_time_fact(
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

fn evidence_valid_from_epoch(conn: &Connection, evidence_event_ids: &[i64]) -> Result<i64> {
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

fn insert_lesson_metadata(
    conn: &Connection,
    memory_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch)
         VALUES (?1, ?2, 1, ?3, ?4, NULL)",
        params![memory_id, candidate.confidence, evidence_json, now],
    )?;
    Ok(())
}

fn refresh_memory_entities(conn: &Connection, id: i64, title: &str, content: &str) -> Result<()> {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    crate::retrieval::entity::refresh_memory_entities(conn, id, &entities)
        .with_context(|| format!("entity refresh failed for memory id={id}"))
}

fn soft_supersede_routed(
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
