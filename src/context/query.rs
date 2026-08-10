use std::collections::HashSet;
use std::time::Instant;

use anyhow::Result;
use rusqlite::Connection;

use super::abstention::filter_recent_rows_by_task_embedding;
use super::commit_signals::query_recent_commit_messages;
use super::filters::{
    push_context_related_filter, push_excluded_type_filter, push_owner_excluded_filter,
    push_owner_included_filter,
};
use super::hybrid_context::{
    query_hybrid_context_memories, query_hybrid_context_memories_with_weights,
};
use super::implicit_query::build_implicit_context_query;
use super::memory_selection::{preselect_memories, sort_memories_by_branch};
use super::ownership::{startup_memory_owner_decision, OwnerCounts, OwnerMetadata, OwnerTrace};
use super::policy::{ContextPolicy, SectionKind};
#[cfg(test)]
pub(super) use super::summary_query::query_recent_summaries;
use super::summary_query::query_recent_summaries_with_drops;
use super::types::{ContextLoadError, ContextPreselectionDrop, LoadedContext, SessionSummaryBrief};
use crate::memory::{self, Memory};
use crate::retrieval::search::SearchWeights;

#[derive(Clone, Copy)]
struct ContextLoadExecutionPolicy {
    allow_remote_embedding: bool,
    allow_rerank: bool,
    fixed_bundle_weights: bool,
}

const DEFAULT_EXECUTION_POLICY: ContextLoadExecutionPolicy = ContextLoadExecutionPolicy {
    allow_remote_embedding: true,
    allow_rerank: true,
    fixed_bundle_weights: false,
};

const LOCAL_ONLY_EXECUTION_POLICY: ContextLoadExecutionPolicy = ContextLoadExecutionPolicy {
    allow_remote_embedding: false,
    allow_rerank: false,
    fixed_bundle_weights: true,
};

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
    load_context_data_with_execution_policy(
        conn,
        project,
        current_branch,
        policy,
        collect_diagnostics,
        DEFAULT_EXECUTION_POLICY,
    )
}

pub(super) fn load_context_data_with_policy_local_only(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    policy: &ContextPolicy,
    collect_diagnostics: bool,
) -> LoadedContext {
    load_context_data_with_execution_policy(
        conn,
        project,
        current_branch,
        policy,
        collect_diagnostics,
        LOCAL_ONLY_EXECUTION_POLICY,
    )
}

fn load_context_data_with_execution_policy(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    policy: &ContextPolicy,
    collect_diagnostics: bool,
    execution_policy: ContextLoadExecutionPolicy,
) -> LoadedContext {
    let render_reference_epoch = chrono::Utc::now().timestamp();
    let mut errors = Vec::new();
    let summary_selection =
        query_recent_summaries_with_drops(conn, project, policy.limits.candidate_fetch_limit)
            .unwrap_or_else(|e| {
                let message = format!("failed to load recent summaries for {project}: {e}");
                crate::log::error("context", &message);
                errors.push(ContextLoadError::new("sessions", message));
                super::summary_query::SummarySelection {
                    selected: Vec::new(),
                    poisoning_drops: Vec::new(),
                    preselection_drops: Vec::new(),
                }
            });
    let summaries = summary_selection.selected;
    let workstreams =
        crate::workstream::query_active_workstreams(conn, project).unwrap_or_else(|e| {
            let message = format!("failed to load active workstreams for {project}: {e}");
            crate::log::error("context", &message);
            errors.push(ContextLoadError::new("workstreams", message));
            Vec::new()
        });
    // Workstream text participates in the implicit retrieval query. Reject
    // instruction-shaped rows before query derivation so poisoned content
    // cannot steer which memories are fetched.
    let (workstreams, poisoned_workstreams) = super::poisoning::partition_workstreams(workstreams);
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
        execution_policy,
        &commit_messages,
        &summaries,
        &workstreams,
    );
    errors.append(&mut memory_selection.errors);
    let relevance_query = memory_selection.fact_label_query.clone();
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
        policy.limits.candidate_fetch_limit as i64,
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
    let staleness_start = Instant::now();
    let staleness_labels = load_staleness_labels(
        conn,
        &staleness_memories,
        render_reference_epoch,
        &mut errors,
    );
    let load_phase_timings = vec![crate::perf::PhaseTiming::elapsed(
        "load_staleness_labels",
        staleness_start,
    )];

    // Shared final rerank stage (GH-851): runs after baseline assembly so no
    // later sort can override it. Off/failure preserves the baseline. The MCP
    // bundle disables this stage because v1 does not plan or hash reranking.
    let rerank = if execution_policy.allow_rerank {
        let verify_before_trust_ids: HashSet<i64> = staleness_labels
            .iter()
            .filter(|(_, label)| label.source_anchor == "verify-before-trust")
            .map(|(id, _)| *id)
            .collect();
        match crate::retrieval::rerank::apply_with_vbt(
            relevance_query.as_deref(),
            &mut memories,
            &verify_before_trust_ids,
        ) {
            Ok(outcome) => {
                let requested = outcome.disabled_reason()
                    != Some(crate::retrieval::rerank::RerankDisabledReason::Off);
                Some(outcome.to_explain(requested))
            }
            Err(error) => {
                let message = format!("rerank stage failed for {project}: {error}");
                crate::log::error("context", &message);
                errors.push(ContextLoadError::new("rerank", message));
                None
            }
        }
    } else {
        None
    };

    LoadedContext {
        render_reference_epoch,
        memories,
        staleness_labels,
        lessons,
        summaries,
        workstreams,
        preselection_drops: summary_selection
            .preselection_drops
            .into_iter()
            .chain(memory_selection.preselection_drops)
            .collect(),
        poisoning_drops: super::poisoning::PoisoningDrops {
            summaries: summary_selection.poisoning_drops,
            workstreams: poisoned_workstreams,
            ..super::poisoning::PoisoningDrops::default()
        },
        relevance_query,
        memory_abstained: memory_selection.abstained,
        errors,
        owner_traces: memory_selection.owner_traces,
        owner_counts: memory_selection.owner_counts,
        diagnostics: memory_selection.diagnostics,
        rerank,
        load_phase_timings,
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
    preselection_drops: Vec<ContextPreselectionDrop>,
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
    execution_policy: ContextLoadExecutionPolicy,
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
        let retrieved = if execution_policy.fixed_bundle_weights {
            query_hybrid_context_memories_with_weights(
                conn,
                project,
                &implicit_query,
                current_branch,
                excluded_types,
                policy.limits.candidate_fetch_limit as i64,
                SearchWeights::context_bundle_v1(),
                execution_policy.allow_remote_embedding,
            )
        } else {
            query_hybrid_context_memories(
                conn,
                project,
                &implicit_query,
                current_branch,
                excluded_types,
                policy.limits.candidate_fetch_limit as i64,
                execution_policy.allow_remote_embedding,
            )
        };
        match retrieved {
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
                execution_policy.allow_remote_embedding,
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
    let preselection = preselect_memories(
        memories,
        current_branch,
        policy.limits.self_diagnostic_limit,
    );
    let hidden_duplicate_groups = preselection.hidden_duplicate_groups;
    let mut selected = preselection.selected;
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
        owner_counts.add_scope(row.owner.owner_scope.as_deref());
        // Trace rows are debug-only, but `owner_counts` above feeds
        // ContextRenderStats and `remem context --status`, so the query itself
        // stays unconditional (it is an indexed `id IN (...)` lookup). Only the
        // per-row trace construction is gated (GH951).
        if collect_diagnostics {
            let decision = startup_memory_owner_decision(
                project,
                &row.memory.project,
                &row.memory.scope,
                &row.owner,
            );
            traces.push(OwnerTrace::memory(
                row.memory.id,
                &row.memory.title,
                &row.owner,
                true,
                decision.reason,
            ));
        }
    }

    // The exclusion query is a `NOT ... OR ...` scan that no index can serve,
    // and its only consumers are debug output and `governance_eval_snapshot`,
    // which passes `collect_diagnostics = true`. Skipping it removes a full
    // table scan from every non-debug SessionStart (GH951).
    if collect_diagnostics {
        let excluded = query_owner_exclusion_traces(conn, project, excluded_types, 30)
            .unwrap_or_else(|e| {
                crate::log::error(
                    "context",
                    &format!("failed to load owner exclusion trace rows for {project}: {e}"),
                );
                Vec::new()
            });
        traces.extend(excluded);
    }

    ContextMemorySelection {
        memories: selected,
        abstained,
        errors,
        owner_traces: traces,
        owner_counts,
        fact_label_query,
        preselection_drops: preselection.drops,
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
    push_owner_included_filter(conn, project, &mut idx, &mut conditions, &mut params)?;
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
         ORDER BY updated_at_epoch DESC, id ASC LIMIT ?{}",
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
        "SELECT {}, {} FROM memories WHERE id IN ({}) ORDER BY updated_at_epoch DESC, id ASC",
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
    push_context_related_filter(conn, project, &mut idx, &mut conditions, &mut params)?;
    push_owner_excluded_filter(conn, project, &mut idx, &mut conditions, &mut params)?;
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
         ORDER BY updated_at_epoch DESC, id ASC LIMIT ?{}",
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
