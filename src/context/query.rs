use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use super::abstention::filter_recent_rows_by_task_embedding;
use super::commit_signals::query_recent_commit_messages;
use super::filters::{
    push_context_related_filter, push_excluded_type_filter, push_owner_excluded_filter,
    push_owner_included_filter,
};
use super::hybrid_context::query_hybrid_context_memories;
use super::implicit_query::build_implicit_context_query;
use super::memory_selection::{
    context_cluster_suffix, deduplicate_memory_clusters, limit_self_diagnostic_memories,
    normalize_cluster_text, reference_cluster_key, sort_memories_by_branch,
};
use super::memory_traits::is_self_diagnostic_text;
use super::ownership::{startup_memory_owner_decision, OwnerCounts, OwnerMetadata, OwnerTrace};
use super::policy::{ContextPolicy, SectionKind};
use super::types::{ContextLoadError, LoadedContext, SessionSummaryBrief};
use crate::memory::{self, Memory};

const SUMMARY_FETCH_BATCH_SIZE: usize = 25;
const SUMMARY_MAX_SCAN: usize = 200;
const STALE_DESIGN_SUMMARY_DAYS: i64 = 7;

#[cfg(test)]
pub(super) fn load_context_data(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
) -> LoadedContext {
    let policy = ContextPolicy::from_limits(super::policy::ContextLimits::default());
    load_context_data_with_policy(conn, project, current_branch, &policy, true)
}

pub(super) fn load_context_data_with_policy(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    policy: &ContextPolicy,
    collect_diagnostics: bool,
) -> LoadedContext {
    let render_reference_epoch = chrono::Utc::now().timestamp();
    let mut errors = Vec::new();
    let summaries = query_recent_summaries(conn, project, policy.limits.session_limit)
        .unwrap_or_else(|e| {
            let message = format!("failed to load recent summaries for {project}: {e}");
            crate::log::error("context", &message);
            errors.push(ContextLoadError::new("sessions", message));
            Vec::new()
        });
    let workstreams =
        crate::workstream::query_active_workstreams(conn, project).unwrap_or_else(|e| {
            let message = format!("failed to load active workstreams for {project}: {e}");
            crate::log::error("context", &message);
            errors.push(ContextLoadError::new("workstreams", message));
            Vec::new()
        });
    let commit_messages = query_recent_commit_messages(conn, project, current_branch, 3)
        .unwrap_or_else(|e| {
            let message = format!("failed to load recent git commit messages for {project}: {e}");
            crate::log::error("context", &message);
            errors.push(ContextLoadError::new("commits", message));
            Vec::new()
        });
    let mut memory_selection = load_project_memories(
        conn,
        project,
        current_branch,
        policy,
        collect_diagnostics,
        &commit_messages,
        &summaries,
        &workstreams,
    );
    errors.append(&mut memory_selection.errors);
    let mut memories = memory_selection.memories;
    sort_memories_by_branch(&mut memories, current_branch);
    if let Err(e) = super::fact_labels::annotate_memories_with_temporal_facts_for_query(
        conn,
        &mut memories,
        memory_selection.fact_label_query.as_deref(),
        Some(project),
    ) {
        let message = format!("failed to load temporal fact labels for {project}: {e}");
        crate::log::error("context", &message);
        errors.push(ContextLoadError::new("memories", message));
    }
    let lessons = memory::lesson::list_lessons_for_context(
        conn,
        project,
        current_branch,
        policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit) as i64,
    )
    .unwrap_or_else(|e| {
        let message = format!("failed to load lessons for {project}: {e}");
        crate::log::error("context", &message);
        errors.push(ContextLoadError::new("lessons", message));
        Vec::new()
    });
    let staleness_memories = memories
        .iter()
        .chain(lessons.iter().map(|lesson| &lesson.memory))
        .cloned()
        .collect::<Vec<_>>();
    let staleness_labels = load_staleness_labels(
        conn,
        &staleness_memories,
        render_reference_epoch,
        &mut errors,
    );

    LoadedContext {
        render_reference_epoch,
        memories,
        staleness_labels,
        lessons,
        summaries,
        workstreams,
        memory_abstained: memory_selection.abstained,
        errors,
        owner_traces: memory_selection.owner_traces,
        owner_counts: memory_selection.owner_counts,
        diagnostics: memory_selection.diagnostics,
    }
}

fn load_staleness_labels(
    conn: &Connection,
    memories: &[Memory],
    now_epoch: i64,
    errors: &mut Vec<ContextLoadError>,
) -> std::collections::HashMap<i64, memory::MemoryStalenessLabel> {
    memory::staleness::memory_staleness_labels_for_memories_lossy(
        conn,
        memories,
        now_epoch,
        |id, error| {
            let message = format!("source-anchor staleness label failed for memory {id}: {error}");
            crate::log::error("context", &message);
            errors.push(ContextLoadError::new("staleness", message));
        },
    )
    .unwrap_or_else(|error| {
        let message = format!("source-anchor staleness batch failed: {error}");
        crate::log::error("context", &message);
        errors.push(ContextLoadError::new("staleness", message));
        memories
            .iter()
            .map(|memory| {
                (
                    memory.id,
                    memory::memory_staleness_error_label(memory, now_epoch, &error),
                )
            })
            .collect()
    })
}

struct ContextMemorySelection {
    memories: Vec<Memory>,
    abstained: bool,
    errors: Vec<ContextLoadError>,
    owner_traces: Vec<OwnerTrace>,
    owner_counts: OwnerCounts,
    diagnostics: super::types::ContextDiagnostics,
    fact_label_query: Option<String>,
}

pub(super) struct ContextMemoryRow {
    pub(super) memory: Memory,
    owner: OwnerMetadata,
}

fn load_project_memories(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    policy: &ContextPolicy,
    collect_diagnostics: bool,
    commit_messages: &[String],
    summaries: &[SessionSummaryBrief],
    workstreams: &[crate::workstream::WorkStream],
) -> ContextMemorySelection {
    let mut memories = Vec::new();
    let mut errors = Vec::new();
    let mut traces = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut abstained = false;
    let mut task_abstention_query = None;
    let mut fact_label_query = None;

    let excluded_types = policy
        .section(SectionKind::MemoryIndex)
        .map(|section| section.exclude_types.as_slice())
        .unwrap_or(&[]);
    let has_task_signals =
        !commit_messages.is_empty() || !summaries.is_empty() || !workstreams.is_empty();
    if let Some(implicit_query) = build_implicit_context_query(
        project,
        current_branch,
        commit_messages,
        summaries,
        workstreams,
    ) {
        fact_label_query = Some(implicit_query.clone());
        match query_hybrid_context_memories(
            conn,
            project,
            &implicit_query,
            current_branch,
            excluded_types,
            policy.limits.candidate_fetch_limit as i64,
        ) {
            Ok(retrieved) => {
                if retrieved.is_empty() && has_task_signals {
                    task_abstention_query = Some(implicit_query);
                } else {
                    for memory in retrieved {
                        if seen_ids.insert(memory.id) {
                            memories.push(memory);
                        }
                    }
                }
            }
            Err(e) => {
                let message =
                    format!("failed to retrieve hybrid context memories for {project}: {e}");
                crate::log::error("context", &message);
                errors.push(ContextLoadError::new("memories", message));
            }
        }
    }

    if !abstained {
        let recent_limit = if task_abstention_query.is_some() {
            policy
                .limits
                .candidate_fetch_limit
                .saturating_mul(20)
                .max(30) as i64
        } else {
            policy.limits.candidate_fetch_limit as i64
        };
        let recent = query_owner_included_memory_rows(
            conn,
            project,
            None,
            current_branch,
            excluded_types,
            recent_limit,
        )
        .unwrap_or_else(|e| {
            let message = format!("failed to load recent context memories for {project}: {e}");
            crate::log::error("context", &message);
            errors.push(ContextLoadError::new("memories", message));
            Vec::new()
        });
        let recent = match task_abstention_query.as_deref() {
            Some(task_query) => filter_recent_rows_by_task_embedding(
                conn,
                task_query,
                recent,
                policy.limits.candidate_fetch_limit,
            )
            .unwrap_or_else(|e| {
                let message =
                    format!("failed to evaluate abstention rescue memories for {project}: {e}");
                crate::log::error("context", &message);
                errors.push(ContextLoadError::new("memories", message));
                abstained = true;
                Vec::new()
            }),
            None => recent,
        };
        if task_abstention_query.is_some() && recent.is_empty() {
            abstained = true;
        }
        for row in recent {
            if seen_ids.insert(row.memory.id) {
                memories.push(row.memory);
            }
        }
    }

    memories
        .retain(|memory| policy.allows_memory_type(SectionKind::MemoryIndex, &memory.memory_type));
    let (deduped, hidden_duplicate_groups) = deduplicate_memory_clusters(memories, current_branch);
    let mut selected = limit_self_diagnostic_memories(deduped, policy.limits.self_diagnostic_limit);
    sort_memories_by_branch(&mut selected, current_branch);
    let selected_id_list = selected.iter().map(|memory| memory.id).collect::<Vec<_>>();

    let selected_ids = selected
        .iter()
        .map(|memory| memory.id)
        .collect::<HashSet<_>>();
    let selected_rows = query_owner_traces_for_ids(conn, &selected_ids).unwrap_or_else(|e| {
        crate::log::error(
            "context",
            &format!("failed to load owner trace rows for {project}: {e}"),
        );
        Vec::new()
    });
    let mut owner_counts = OwnerCounts::default();
    for row in selected_rows {
        let decision = startup_memory_owner_decision(
            project,
            &row.memory.project,
            &row.memory.scope,
            &row.owner,
        );
        owner_counts.add_scope(row.owner.owner_scope.as_deref());
        traces.push(OwnerTrace::memory(
            row.memory.id,
            &row.memory.title,
            &row.owner,
            true,
            decision.reason,
        ));
    }

    let excluded =
        query_owner_exclusion_traces(conn, project, excluded_types, 30).unwrap_or_else(|e| {
            crate::log::error(
                "context",
                &format!("failed to load owner exclusion trace rows for {project}: {e}"),
            );
            Vec::new()
        });
    traces.extend(excluded);

    ContextMemorySelection {
        memories: selected,
        abstained,
        errors,
        owner_traces: traces,
        owner_counts,
        fact_label_query,
        diagnostics: if collect_diagnostics {
            super::diagnostics::collect_context_diagnostics(
                conn,
                project,
                excluded_types,
                selected_id_list,
                hidden_duplicate_groups,
            )
        } else {
            super::types::ContextDiagnostics::default()
        },
    }
}

fn query_owner_included_memory_rows(
    conn: &Connection,
    project: &str,
    query: Option<&str>,
    current_branch: Option<&str>,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<ContextMemoryRow>> {
    if limit <= 0 || query.is_some_and(|value| value.trim().is_empty()) {
        return Ok(vec![]);
    }

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    push_owner_included_filter(project, &mut idx, &mut conditions, &mut params);
    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        false,
    ));
    conditions.push(crate::memory::memory_state_key_current_filter_sql(
        "memories",
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        "memories",
    ));
    if let Some(branch) = current_branch.filter(|branch| !branch.trim().is_empty()) {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }

    if let Some(query) = query {
        let like_pattern = format!("%{query}%");
        conditions.push(format!("(title LIKE ?{idx} OR content LIKE ?{idx})"));
        params.push(Box::new(like_pattern));
        idx += 1;
    }

    push_excluded_type_filter(excluded_types, &mut idx, &mut conditions, &mut params);
    params.push(Box::new(limit));
    let sql = format!(
        "SELECT {}, {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{}",
        memory::MEMORY_COLS,
        MEMORY_OWNER_COLS,
        conditions.join(" AND "),
        idx,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), map_context_memory_row)?;
    crate::db::query::collect_rows(rows)
}

fn query_owner_traces_for_ids(
    conn: &Connection,
    selected_ids: &HashSet<i64>,
) -> Result<Vec<ContextMemoryRow>> {
    if selected_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut ids = selected_ids.iter().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    let placeholders = (1..=ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT {}, {} FROM memories WHERE id IN ({}) ORDER BY updated_at_epoch DESC",
        memory::MEMORY_COLS,
        MEMORY_OWNER_COLS,
        placeholders
    );
    let params = ids
        .into_iter()
        .map(|id| Box::new(id) as Box<dyn rusqlite::types::ToSql>)
        .collect::<Vec<_>>();
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), map_context_memory_row)?;
    crate::db::query::collect_rows(rows)
}

fn query_owner_exclusion_traces(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<OwnerTrace>> {
    if limit <= 0 {
        return Ok(vec![]);
    }

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    push_context_related_filter(project, &mut idx, &mut conditions, &mut params);
    push_owner_excluded_filter(project, &mut idx, &mut conditions, &mut params);
    conditions.push(crate::memory::memory_current_filter_sql(
        "status",
        "expires_at_epoch",
        false,
    ));
    conditions.push(crate::memory::suppression::memory_policy_filter_sql(
        "memories",
    ));
    push_excluded_type_filter(excluded_types, &mut idx, &mut conditions, &mut params);
    params.push(Box::new(limit));

    let sql = format!(
        "SELECT {}, {} FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC LIMIT ?{}",
        memory::MEMORY_COLS,
        MEMORY_OWNER_COLS,
        conditions.join(" AND "),
        idx,
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), |row| {
        let context_row = map_context_memory_row(row)?;
        let decision = startup_memory_owner_decision(
            project,
            &context_row.memory.project,
            &context_row.memory.scope,
            &context_row.owner,
        );
        Ok(OwnerTrace::memory(
            context_row.memory.id,
            &context_row.memory.title,
            &context_row.owner,
            false,
            decision.reason,
        ))
    })?;
    crate::db::query::collect_rows(rows)
}

const MEMORY_OWNER_COLS: &str = "source_project, target_project, owner_scope, owner_key, \
                                topic_domain, context_class";

fn map_context_memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextMemoryRow> {
    Ok(ContextMemoryRow {
        memory: memory::map_memory_row_pub(row)?,
        owner: OwnerMetadata::from_memory_row(row, 13)?,
    })
}

pub(super) fn query_recent_summaries(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<SessionSummaryBrief>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let scan_limit = SUMMARY_MAX_SCAN.max(limit);
    let now_epoch = chrono::Utc::now().timestamp();
    let mut selected = Vec::new();
    let mut low_signal_fallback = Vec::new();
    let mut seen_clusters = HashSet::new();
    let mut offset = 0usize;

    while selected.len() < limit && offset < scan_limit {
        let fetch_limit = SUMMARY_FETCH_BATCH_SIZE.min(scan_limit - offset);
        let batch = query_summary_batch(conn, project, fetch_limit, offset)?;
        if batch.is_empty() {
            break;
        }

        for summary in batch {
            if is_session_summary_self_diagnostic(&summary) {
                continue;
            }

            let cluster_key = summary_cluster_key(&summary);
            if seen_clusters.contains(&cluster_key) {
                continue;
            }

            if is_stale_design_prototype_summary(&summary, now_epoch) {
                low_signal_fallback.push((cluster_key, summary));
                continue;
            }

            seen_clusters.insert(cluster_key);
            selected.push(summary);
            if selected.len() >= limit {
                break;
            }
        }

        offset += fetch_limit;
    }

    if selected.is_empty() {
        for (cluster_key, summary) in low_signal_fallback {
            if seen_clusters.insert(cluster_key) {
                selected.push(summary);
            }
            if selected.len() >= limit {
                break;
            }
        }
    }

    Ok(selected)
}

fn query_summary_batch(
    conn: &Connection,
    project: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<SessionSummaryBrief>> {
    let mut stmt = conn.prepare(
        "SELECT request, completed, created_at_epoch \
         FROM session_summaries \
         WHERE request IS NOT NULL AND request != '' \
           AND session_row_id IS NULL \
           AND ((owner_scope = 'repo' AND owner_key = ?1) \
                OR (owner_scope = 'repo' AND target_project = ?1) \
                OR (owner_scope IS NULL AND project = ?1)) \
         ORDER BY created_at_epoch DESC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![project, limit as i64, offset as i64],
        |row| {
            Ok(SessionSummaryBrief {
                request: row.get(0)?,
                completed: row.get(1)?,
                created_at_epoch: row.get(2)?,
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
}

fn is_session_summary_self_diagnostic(summary: &SessionSummaryBrief) -> bool {
    let haystack = session_summary_haystack(summary);
    is_self_diagnostic_text(&haystack)
}

fn is_stale_design_prototype_summary(summary: &SessionSummaryBrief, now_epoch: i64) -> bool {
    let age_days = (now_epoch - summary.created_at_epoch) / 86400;
    if age_days <= STALE_DESIGN_SUMMARY_DAYS {
        return false;
    }

    let haystack = session_summary_haystack(summary);
    ["landing page", "wireframe", "starfield"]
        .iter()
        .any(|needle| haystack.contains(needle))
}

fn summary_cluster_key(summary: &SessionSummaryBrief) -> String {
    let request = normalize_cluster_text(&summary.request);
    let tokens: Vec<&str> = request.split_whitespace().collect();
    if let Some(reference_key) = reference_cluster_key(&tokens) {
        return reference_key;
    }
    context_cluster_suffix(&request)
}

fn session_summary_haystack(summary: &SessionSummaryBrief) -> String {
    format!(
        "{} {}",
        summary.request,
        summary.completed.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase()
}
