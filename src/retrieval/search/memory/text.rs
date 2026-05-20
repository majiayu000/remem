use std::collections::HashMap;

use anyhow::{anyhow, Result};
use rusqlite::Connection;

use crate::memory::{self, Memory};
use crate::retrieval::memory_search::ProjectScopeFilter;

use super::super::common::{paginate_memories, rrf_fuse, sanitize_fts_query};

fn load_ordered_memories(conn: &Connection, ids: &[i64]) -> Result<Vec<Memory>> {
    let loaded = memory::get_memories_by_ids(conn, ids, None)?;
    let id_to_memory: HashMap<i64, Memory> = loaded
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect();
    Ok(ids
        .iter()
        .filter_map(|id| id_to_memory.get(id).cloned())
        .collect())
}

pub(super) fn search_with_query(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    search_with_query_scope(
        conn,
        query_text,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        ProjectScopeFilter::IncludeGlobal,
    )
}

pub(super) fn search_project_scoped_with_query(
    conn: &Connection,
    query_text: &str,
    project: &str,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    search_with_query_scope(
        conn,
        query_text,
        Some(project),
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        ProjectScopeFilter::ProjectOnly,
    )
}

#[allow(clippy::too_many_arguments)]
fn search_with_query_scope(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    scope_filter: ProjectScopeFilter,
) -> Result<Vec<Memory>> {
    let project_scoped_name = match scope_filter {
        ProjectScopeFilter::IncludeGlobal => "",
        ProjectScopeFilter::ProjectOnly => {
            project.ok_or_else(|| anyhow!("project-scoped search requires a project"))?
        }
    };
    let page_target = (limit.max(1) + offset.max(0) + 1).max(2);
    let fetch = page_target * 3;
    let expanded = crate::retrieval::query_expand::expand_query(query_text);
    let expanded_refs: Vec<&str> = expanded.iter().map(|token| token.as_str()).collect();
    let long_tokens: Vec<&str> = expanded_refs
        .iter()
        .filter(|token| token.chars().count() >= 3)
        .copied()
        .collect();

    let core_tokens = crate::retrieval::query_expand::core_tokens(query_text);
    let core_refs: Vec<&str> = core_tokens.iter().map(|token| token.as_str()).collect();
    let mut channels: Vec<Vec<i64>> = Vec::new();

    if !long_tokens.is_empty() {
        let safe_query = sanitize_fts_query(&long_tokens.join(" "));
        let fts = match scope_filter {
            ProjectScopeFilter::IncludeGlobal => memory::search_memories_fts_filtered(
                conn,
                &safe_query,
                project,
                memory_type,
                fetch,
                0,
                include_stale,
                branch,
            )?,
            ProjectScopeFilter::ProjectOnly => memory::search_project_memories_fts_filtered(
                conn,
                &safe_query,
                project_scoped_name,
                memory_type,
                fetch,
                0,
                include_stale,
                branch,
            )?,
        };
        channels.push(fts.iter().map(|memory| memory.id).collect());
    }

    let entity_ids = match scope_filter {
        ProjectScopeFilter::IncludeGlobal => crate::retrieval::entity::search_by_entity_filtered(
            conn,
            query_text,
            project,
            memory_type,
            branch,
            fetch,
            include_stale,
        )?,
        ProjectScopeFilter::ProjectOnly => {
            crate::retrieval::entity::search_project_memories_by_entity_filtered(
                conn,
                query_text,
                project_scoped_name,
                memory_type,
                branch,
                fetch,
                include_stale,
            )?
        }
    };
    if !entity_ids.is_empty() {
        channels.push(entity_ids);
    }

    if let Some(temporal_constraint) = crate::retrieval::temporal::extract_temporal(query_text) {
        let temporal_ids = match scope_filter {
            ProjectScopeFilter::IncludeGlobal => {
                crate::retrieval::temporal::search_by_time_filtered(
                    conn,
                    &temporal_constraint,
                    project,
                    memory_type,
                    branch,
                    fetch,
                    include_stale,
                )?
            }
            ProjectScopeFilter::ProjectOnly => {
                crate::retrieval::temporal::search_by_time_project_scoped(
                    conn,
                    &temporal_constraint,
                    project_scoped_name,
                    memory_type,
                    branch,
                    fetch,
                    include_stale,
                )?
            }
        };
        if !temporal_ids.is_empty() {
            channels.push(temporal_ids);
        }
    }

    let like = match scope_filter {
        ProjectScopeFilter::IncludeGlobal => memory::search_memories_like_filtered(
            conn,
            &core_refs,
            project,
            memory_type,
            fetch,
            0,
            include_stale,
            branch,
        )?,
        ProjectScopeFilter::ProjectOnly => memory::search_project_memories_like_filtered(
            conn,
            &core_refs,
            project_scoped_name,
            memory_type,
            fetch,
            0,
            include_stale,
            branch,
        )?,
    };
    if !like.is_empty() {
        channels.push(like.iter().map(|memory| memory.id).collect());
    }

    if channels.is_empty() {
        return Ok(vec![]);
    }

    let final_ids: Vec<i64> = rrf_fuse(&channels, 60.0)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let ordered = load_ordered_memories(conn, &final_ids)?;
    Ok(paginate_memories(ordered, limit, offset))
}
