use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use super::{candidate_title, normalize_evidence_text, CandidateRoute, ParsedMemoryCandidate};

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
    let active = find_active_same_topic(conn, candidate, route)?;
    if let Some(existing) = active.iter().find(|row| {
        normalize_evidence_text(&row.content) == normalize_evidence_text(&candidate.text)
    }) {
        return Ok(CandidateApplyOutcome {
            memory_id: Some(existing.id),
            promoted: false,
            noop: true,
            superseded: 0,
        });
    }

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
    )?;
    let superseded_ids = active.iter().map(|row| row.id).collect::<Vec<_>>();
    let superseded = soft_supersede_routed(conn, &superseded_ids, Some(memory_id))?;
    Ok(CandidateApplyOutcome {
        memory_id: Some(memory_id),
        promoted: true,
        noop: false,
        superseded,
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
             context_class = ?13
         WHERE id = ?14",
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
            candidate_id
        ],
    )?;
    Ok(())
}

#[derive(Debug)]
struct ActiveTopicMemory {
    id: i64,
    content: String,
}

fn find_active_same_topic(
    conn: &Connection,
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
) -> Result<Vec<ActiveTopicMemory>> {
    let mut stmt = conn.prepare(
        "SELECT id, content
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
            route.owner_key
        ],
        |row| {
            Ok(ActiveTopicMemory {
                id: row.get(0)?,
                content: row.get(1)?,
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
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
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let search_context = crate::memory::search_context::build_search_context(
        &candidate.memory_type,
        Some(&candidate.topic_key),
        &candidate.text,
        None,
    );
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, status, branch, scope,
          evidence_event_ids, source_candidate_id, confidence,
          source_project, target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, routing_reason, context_class)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7,
                 ?8, ?8, 'active', NULL, ?9,
                 ?10, ?11, ?12,
                 ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            session_id,
            memory_project,
            candidate.topic_key,
            title,
            candidate.text,
            candidate.memory_type,
            search_context,
            now,
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
            route.context_class
        ],
    )?;
    let memory_id = conn.last_insert_rowid();
    if candidate.memory_type == "lesson" {
        insert_lesson_metadata(conn, memory_id, candidate, evidence_json, now)?;
    }
    refresh_memory_entities(conn, memory_id, title, &candidate.text);
    Ok(memory_id)
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

fn refresh_memory_entities(conn: &Connection, id: i64, title: &str, content: &str) {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    if entities.is_empty() {
        return;
    }
    if let Err(e) = crate::retrieval::entity::link_entities(conn, id, &entities) {
        crate::log::warn("memory", &format!("entity link failed for id={id}: {e}"));
    }
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
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            params![id],
        )?;
        if updated != 1 {
            bail!("failed to mark superseded memory stale: id={id}");
        }
        changed += updated;
    }
    Ok(changed)
}
