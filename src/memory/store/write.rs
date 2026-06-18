use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::search_context::build_search_context;
use crate::memory::state_key::{self, StateKeyDecision};
use crate::memory::{
    lifecycle::MemoryLifecycleOp,
    operation::{
        insert_operation_log, with_operation_savepoint, MemoryOperationInput, MemoryOperationPlan,
    },
    preference::consolidation::PreferenceConsolidationKind,
};

pub fn insert_memory(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
) -> Result<i64> {
    insert_memory_with_branch(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        None,
    )
}

pub fn insert_memory_with_branch(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
) -> Result<i64> {
    insert_memory_full(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        branch,
        "project",
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_memory_full(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    created_at_override: Option<i64>,
) -> Result<i64> {
    insert_memory_full_with_reference_time(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        branch,
        scope,
        created_at_override,
        created_at_override,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_memory_full_with_reference_time(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    created_at_override: Option<i64>,
    reference_time_override: Option<i64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let created_at = created_at_override.unwrap_or(now);
    let reference_time = reference_time_override
        .or(created_at_override)
        .unwrap_or(created_at);
    let (expires_at_epoch, valid_from_epoch) =
        crate::memory::lifecycle::ttl_metadata(memory_type, topic_key, content, now);
    let search_context = build_search_context(memory_type, topic_key, content, files);
    let ownership = default_ownership(project, scope);
    let state_key = state_key::derive_state_key(memory_type, topic_key, title, content);

    let mut existing_id = None;
    let mut preference_conflict = false;
    if let Some(topic_key) = topic_key {
        if !topic_key.is_empty() {
            existing_id = conn
                .query_row(
                    "SELECT id FROM memories
                     WHERE project = ?1 AND topic_key = ?2 AND scope = ?3
                     ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                              updated_at_epoch DESC,
                              id DESC
                     LIMIT 1",
                    params![project, topic_key, scope],
                    |row| row.get(0),
                )
                .optional()?;
        }
    }

    if existing_id.is_none() {
        if let Some(decision) = &state_key {
            if decision.allows_direct_upsert() {
                existing_id = state_key::current_memory_id(
                    conn,
                    ownership.owner_scope,
                    ownership.owner_key,
                    memory_type,
                    &decision.state_key,
                    now,
                )?;
            }
        }
    }
    if memory_type == "preference" && existing_id.is_none() {
        if let Some(preference_match) =
            crate::memory::preference::consolidation::find_preference_consolidation(
                conn,
                ownership.owner_scope,
                ownership.owner_key,
                scope,
                branch,
                content,
                now,
            )?
        {
            match preference_match.kind {
                PreferenceConsolidationKind::SamePreference
                | PreferenceConsolidationKind::Refinement => {
                    existing_id = Some(preference_match.memory_id);
                }
                PreferenceConsolidationKind::Contradiction => {
                    preference_conflict = true;
                }
            }
        }
    }

    if existing_id.is_none() && !preference_conflict {
        existing_id = crate::memory::semantic_dedup::find_curated_duplicate_id(
            conn,
            project,
            scope,
            memory_type,
            title,
            content,
            topic_key,
            branch,
            now,
        )?;
    }

    if let Some(id) = existing_id {
        return with_memory_savepoint(conn, || {
            update_existing_memory(
                conn,
                id,
                session_id,
                topic_key,
                title,
                content,
                memory_type,
                files,
                branch,
                scope,
                &search_context,
                expires_at_epoch,
                valid_from_epoch,
                &ownership,
                state_key.as_ref(),
                now,
                reference_time,
            )?;
            refresh_memory_entities(conn, id, title, content)?;
            refresh_memory_embedding(conn, id, title, content, memory_type, topic_key)?;
            Ok(id)
        });
    }

    with_memory_savepoint(conn, || {
        conn.execute(
            "INSERT INTO memories \
             (session_id, project, topic_key, title, content, memory_type, files, search_context, \
              created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope, \
              source_project, target_project, owner_scope, owner_key, context_class, \
              expires_at_epoch, valid_from_epoch) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13, \
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                session_id,
                project,
                topic_key,
                title,
                content,
                memory_type,
                files,
                search_context,
                created_at,
                now,
                reference_time,
                branch,
                scope,
                ownership.source_project,
                ownership.target_project,
                ownership.owner_scope,
                ownership.owner_key,
                ownership.context_class,
                expires_at_epoch,
                valid_from_epoch
            ],
        )?;
        let id = conn.last_insert_rowid();
        attach_state_key(conn, id, memory_type, &ownership, state_key.as_ref(), now)?;
        refresh_memory_entities(conn, id, title, content)?;
        refresh_memory_embedding(conn, id, title, content, memory_type, topic_key)?;
        Ok(id)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn insert_memory_full_with_operation_log(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    created_at_override: Option<i64>,
    reference_time_override: Option<i64>,
    operation_input: &MemoryOperationInput,
    operation_plan: &MemoryOperationPlan,
) -> Result<(i64, MemoryLifecycleOp)> {
    with_operation_savepoint(conn, || {
        let id = insert_memory_full_with_reference_time(
            conn,
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            branch,
            scope,
            created_at_override,
            reference_time_override,
        )?;
        let mut logged_plan = operation_plan.clone();
        logged_plan.target_memory_id = Some(id);
        let operation_id = insert_operation_log(conn, operation_input, &logged_plan, Some(id))?;
        crate::memory::edge::insert_supersedes_edges(
            conn,
            &logged_plan.superseded_ids,
            id,
            crate::memory::edge::MemoryEdgeWriteContext {
                source_candidate_id: operation_input.source_candidate_id,
                source_operation_id: Some(operation_id),
                confidence: operation_input.confidence,
                reason: Some(logged_plan.reason.as_str()),
                ..Default::default()
            },
        )?;
        crate::memory::edge::insert_conflicts_edges(
            conn,
            &logged_plan.conflicting_ids,
            id,
            crate::memory::edge::MemoryEdgeWriteContext {
                source_candidate_id: operation_input.source_candidate_id,
                source_operation_id: Some(operation_id),
                confidence: operation_input.confidence,
                reason: Some(logged_plan.reason.as_str()),
                ..Default::default()
            },
        )?;
        Ok((id, logged_plan.op))
    })
}

pub(crate) struct DefaultOwnership<'a> {
    pub(crate) source_project: &'a str,
    pub(crate) target_project: Option<&'a str>,
    pub(crate) owner_scope: &'static str,
    pub(crate) owner_key: &'a str,
    pub(crate) context_class: &'static str,
}

pub(crate) fn default_ownership<'a>(project: &'a str, scope: &str) -> DefaultOwnership<'a> {
    if scope == "global" {
        DefaultOwnership {
            source_project: project,
            target_project: None,
            owner_scope: "user",
            owner_key: "user:default",
            context_class: "startup_core",
        }
    } else {
        DefaultOwnership {
            source_project: project,
            target_project: Some(project),
            owner_scope: "repo",
            owner_key: project,
            context_class: "startup_core",
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update_existing_memory(
    conn: &Connection,
    id: i64,
    session_id: Option<&str>,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    search_context: &str,
    expires_at_epoch: Option<i64>,
    valid_from_epoch: Option<i64>,
    ownership: &DefaultOwnership<'_>,
    state_key: Option<&StateKeyDecision>,
    now: i64,
    reference_time: i64,
) -> Result<()> {
    let state_key_id = attach_state_key(conn, id, memory_type, ownership, state_key, now)?;
    clear_obsolete_state_key_links(conn, id, state_key_id, now)?;
    conn.execute(
        "UPDATE memories SET session_id = ?1, topic_key = ?2, title = ?3, content = ?4, \
         memory_type = ?5, files = ?6, updated_at_epoch = ?7, branch = ?8, \
         scope = ?9, search_context = ?10, reference_time_epoch = ?11, \
         status = 'active', valid_to_epoch = NULL, \
         expires_at_epoch = ?12, valid_from_epoch = ?13, \
         state_key_id = ?14, \
         source_project = COALESCE(source_project, ?15), \
         target_project = COALESCE(target_project, ?16), \
         owner_scope = COALESCE(owner_scope, ?17), \
         owner_key = COALESCE(owner_key, ?18), \
         context_class = COALESCE(context_class, ?19) \
         WHERE id = ?20",
        params![
            session_id,
            topic_key,
            title,
            content,
            memory_type,
            files,
            now,
            branch,
            scope,
            search_context,
            reference_time,
            expires_at_epoch,
            valid_from_epoch,
            state_key_id,
            ownership.source_project,
            ownership.target_project,
            ownership.owner_scope,
            ownership.owner_key,
            ownership.context_class,
            id
        ],
    )?;
    Ok(())
}

fn clear_obsolete_state_key_links(
    conn: &Connection,
    id: i64,
    active_state_key_id: Option<i64>,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE memory_state_keys
         SET current_memory_id = NULL, updated_at_epoch = ?3
         WHERE current_memory_id = ?1
           AND (?2 IS NULL OR id <> ?2)",
        params![id, active_state_key_id, now],
    )?;
    Ok(())
}

fn attach_state_key(
    conn: &Connection,
    id: i64,
    memory_type: &str,
    ownership: &DefaultOwnership<'_>,
    state_key: Option<&StateKeyDecision>,
    now: i64,
) -> Result<Option<i64>> {
    state_key
        .map(|decision| {
            state_key::attach_current_memory(
                conn,
                id,
                ownership.owner_scope,
                ownership.owner_key,
                memory_type,
                decision,
                now,
            )
        })
        .transpose()
}

fn with_memory_savepoint<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_memory_state_write")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_memory_state_write")?;
            Ok(value)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_memory_state_write;
                 RELEASE SAVEPOINT remem_memory_state_write;",
            );
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!(
                    "memory state-key rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}

fn refresh_memory_entities(conn: &Connection, id: i64, title: &str, content: &str) -> Result<()> {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    crate::retrieval::entity::refresh_memory_entities(conn, id, &entities)
        .with_context(|| format!("entity refresh failed for memory id={id}"))
}

fn refresh_memory_embedding(
    conn: &Connection,
    id: i64,
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Result<()> {
    crate::retrieval::vector::upsert_memory_embedding(
        conn,
        id,
        title,
        content,
        memory_type,
        topic_key,
    )
}

#[cfg(test)]
mod tests;
