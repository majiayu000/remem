use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

use super::{candidate_title, CandidateRoute, ParsedMemoryCandidate};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{
    insert_operation_log, same_memory_text, with_operation_savepoint, MemoryOperationInput,
    MemoryOperationPlan,
};
use crate::memory::poisoning::SourceTrustClass;
use crate::memory::preference::consolidation::{
    load_active_preference_content, PreferenceConsolidationKind,
};

mod activation_request;
mod dream_supersede;
mod route_filter;
mod write;

use write::{
    evidence_valid_from_epoch, insert_candidate_event_time_fact, insert_routed_memory,
    soft_supersede_routed,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CandidateApplyOutcome {
    pub memory_id: Option<i64>,
    pub promoted: bool,
    pub noop: bool,
    pub superseded: usize,
    pub superseded_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SupersedePolicy {
    Unrestricted,
    RequireExact {
        memory_ids: BTreeSet<i64>,
        provenance_epoch: i64,
    },
}

impl SupersedePolicy {
    fn required_memory_ids(&self) -> Option<&BTreeSet<i64>> {
        match self {
            Self::Unrestricted => None,
            Self::RequireExact { memory_ids, .. } => Some(memory_ids),
        }
    }

    fn ensure_exact(&self, memory_ids: &[i64]) -> Result<()> {
        let Self::RequireExact {
            memory_ids: required,
            ..
        } = self
        else {
            return Ok(());
        };
        let actual = memory_ids.iter().copied().collect::<BTreeSet<_>>();
        if &actual != required {
            bail!(
                "candidate promotion supersede set does not exactly match acknowledged Dream provenance: required={required:?} actual={actual:?}"
            );
        }
        Ok(())
    }

    fn is_unrestricted(&self) -> bool {
        matches!(self, Self::Unrestricted)
    }

    fn provenance_epoch(&self) -> Option<i64> {
        match self {
            Self::Unrestricted => None,
            Self::RequireExact {
                provenance_epoch, ..
            } => Some(*provenance_epoch),
        }
    }
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
    source_trust: SourceTrustClass,
) -> Result<CandidateApplyOutcome> {
    promote_candidate_to_memory_inner(
        conn,
        session_id,
        source_project,
        candidate_id,
        candidate,
        evidence_json,
        route,
        source_trust,
        SupersedePolicy::Unrestricted,
        crate::memory::activation::ActivationActorKind::AutomaticWorker,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn promote_candidate_to_memory_with_route_and_policy(
    conn: &Connection,
    session_id: Option<&str>,
    source_project: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    route: &CandidateRoute,
    source_trust: SourceTrustClass,
    supersede_policy: SupersedePolicy,
    review_binding: &str,
    acknowledged_pattern: Option<(&str, i64)>,
) -> Result<CandidateApplyOutcome> {
    promote_candidate_to_memory_inner(
        conn,
        session_id,
        source_project,
        candidate_id,
        candidate,
        evidence_json,
        route,
        source_trust,
        supersede_policy,
        crate::memory::activation::ActivationActorKind::Operator,
        Some(review_binding),
        acknowledged_pattern,
    )
}

#[allow(clippy::too_many_arguments)]
fn promote_candidate_to_memory_inner(
    conn: &Connection,
    session_id: Option<&str>,
    source_project: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    route: &CandidateRoute,
    source_trust: SourceTrustClass,
    supersede_policy: SupersedePolicy,
    actor_kind: crate::memory::activation::ActivationActorKind,
    review_binding: Option<&str>,
    acknowledged_pattern: Option<(&str, i64)>,
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
        let discovered_active = find_active_same_state_or_topic(
            conn,
            candidate,
            route,
            &memory_project,
            memory_scope,
            state_key.as_ref(),
            now,
            candidate_has_ttl,
        )?;
        let mut active = if let Some(required_ids) = supersede_policy.required_memory_ids() {
            let unexpected_current = discovered_active
                .iter()
                .filter(|row| row.is_current && !required_ids.contains(&row.id))
                .map(|row| row.id)
                .collect::<Vec<_>>();
            if !unexpected_current.is_empty() {
                bail!(
                    "candidate promotion would collide with active memories outside acknowledged Dream provenance: {unexpected_current:?}"
                );
            }
            dream_supersede::load_required_memories(
                conn,
                candidate,
                route,
                &memory_project,
                required_ids,
                now,
                candidate_has_ttl,
            )?
        } else {
            discovered_active
        };
        let mut generic_preference_reason = None;
        let mut conflicting_ids = Vec::new();
        if supersede_policy.is_unrestricted()
            && candidate.memory_type == "preference"
            && active.is_empty()
        {
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
                if route_filter::matches_active_route(
                    conn,
                    preference_match.memory_id,
                    &memory_project,
                    memory_scope,
                    route,
                )? {
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
        }
        if let Some(existing) = supersede_policy
            .is_unrestricted()
            .then(|| {
                active
                    .iter()
                    .filter(|row| row.is_current)
                    .find(|row| same_memory_text(&row.content, &candidate.text))
            })
            .flatten()
        {
            if candidate.memory_type == "preference" {
                crate::memory::preference::reinforcement::reinforce_existing_preference(
                    conn,
                    existing.id,
                    &candidate.text,
                    &candidate.risk_class,
                    Some(evidence_json),
                    now,
                )?;
                crate::memory::preference::compilation::enqueue_for_memory_ids(
                    conn,
                    &[existing.id],
                )?;
            }
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
                superseded_ids: Vec::new(),
            });
        }

        let superseded_ids = active.iter().map(|row| row.id).collect::<Vec<_>>();
        supersede_policy.ensure_exact(&superseded_ids)?;
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
        let reference_time_epoch = if evidence_event_ids.is_empty() {
            supersede_policy
                .provenance_epoch()
                .context("candidate promotion requires evidence_event_ids or reviewed provenance")?
        } else {
            evidence_valid_from_epoch(conn, &evidence_event_ids)?
        };
        let activation_request = activation_request::build(
            source_project,
            &memory_project,
            memory_scope,
            candidate_id,
            candidate,
            evidence_json,
            route,
            source_trust,
            actor_kind,
            &superseded_ids,
            review_binding,
            acknowledged_pattern,
        )?;
        let mut applied_outcome = None;
        let activation = crate::memory::activation::execute_one(conn, &activation_request, |_| {
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
                source_trust,
            )?;
            if let Some((pattern_id, pattern_version)) = acknowledged_pattern {
                conn.execute(
                    "UPDATE memories
                     SET acknowledged_pattern_id = ?1, acknowledged_pattern_version = ?2,
                         acknowledged_at_epoch = ?3
                     WHERE id = ?4",
                    params![pattern_id, pattern_version, now, memory_id],
                )?;
            }
            plan.target_memory_id = Some(memory_id);
            let superseded = soft_supersede_routed(conn, &superseded_ids, Some(memory_id))?;
            if superseded != superseded_ids.len() {
                bail!(
                "candidate promotion supersede write count changed inside transaction: expected={} actual={superseded}",
                superseded_ids.len()
            );
            }
            if candidate.memory_type == "preference" {
                crate::memory::preference::reinforcement::persist_preference_reinforcement(
                    conn,
                    memory_id,
                    &superseded_ids,
                    &candidate.text,
                    &candidate.risk_class,
                    Some(evidence_json),
                    now,
                )?;
                crate::memory::preference::compilation::enqueue_for_memory_ids(conn, &[memory_id])?;
            }
            let operation_id =
                insert_operation_log(conn, &operation_input, &plan, Some(memory_id))?;
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
            super::fact_extract::write_candidate_facts(
                conn,
                &memory_project,
                memory_id,
                &candidate.facts,
                &evidence_event_ids,
                reference_time_epoch,
                candidate.confidence,
            )
            .with_context(|| {
                format!(
                    "failed to write extracted SPO facts for promoted candidate id={candidate_id}"
                )
            })?;
            applied_outcome = Some(CandidateApplyOutcome {
                memory_id: Some(memory_id),
                promoted: true,
                noop: false,
                superseded,
                superseded_ids: superseded_ids.clone(),
            });
            Ok(memory_id)
        })?;
        if activation.replayed {
            return Ok(CandidateApplyOutcome {
                memory_id: Some(activation.memory_id),
                promoted: false,
                noop: true,
                superseded: 0,
                superseded_ids: Vec::new(),
            });
        }
        applied_outcome.context("candidate activation completed without apply outcome")
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
    memory_project: &str,
    memory_scope: &str,
    now_epoch: i64,
    candidate_has_ttl: bool,
) -> Result<Vec<ActiveTopicMemory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content,
                CASE
                    WHEN ?8 = 1 THEN
                        CASE
                            WHEN expires_at_epoch IS NOT NULL AND expires_at_epoch > ?9 THEN 1
                            ELSE 0
                        END
                    WHEN expires_at_epoch IS NULL OR expires_at_epoch > ?9 THEN 1
                    ELSE 0
                END AS is_current
         FROM memories
         WHERE status = 'active'
           AND memory_type = ?1
           AND topic_key = ?2
           AND project = ?3
           AND branch IS NULL
           AND COALESCE(scope, 'project') = ?4
           AND COALESCE(
                owner_scope,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
           ) = ?5
           AND COALESCE(
                owner_key,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
           ) = ?6
           AND CASE
               WHEN COALESCE(owner_scope,
                   CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
               ) = 'repo'
               THEN COALESCE(target_project, project)
               ELSE target_project
           END IS ?7
         ORDER BY updated_at_epoch DESC, id DESC",
    )?;
    let rows = stmt.query_map(
        params![
            candidate.memory_type,
            candidate.topic_key,
            memory_project,
            memory_scope,
            route.owner_scope,
            route.owner_key,
            route.target_project,
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
    memory_project: &str,
    memory_scope: &str,
    state_key: Option<&crate::memory::state_key::StateKeyDecision>,
    now_epoch: i64,
    candidate_has_ttl: bool,
) -> Result<Vec<ActiveTopicMemory>> {
    let mut memories = find_active_same_topic(
        conn,
        candidate,
        route,
        memory_project,
        memory_scope,
        now_epoch,
        candidate_has_ttl,
    )?;
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
        let ids = route_filter::ids(conn, ids, memory_project, memory_scope, route)?;
        for id in ids {
            if !memories.iter().any(|memory| memory.id == id) {
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
        }
    }
    Ok(memories)
}
