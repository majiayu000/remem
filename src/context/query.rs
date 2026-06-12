use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::commit_signals::query_recent_commit_messages;
use super::filters::{
    push_context_related_filter, push_excluded_type_filter, push_owner_excluded_filter,
    push_owner_included_filter,
};
use super::hybrid_context::query_hybrid_context_memories;
use super::implicit_query::build_implicit_context_query;
use super::memory_traits::{is_memory_self_diagnostic, is_self_diagnostic_text};
use super::ownership::{startup_memory_owner_decision, OwnerCounts, OwnerMetadata, OwnerTrace};
use super::policy::{ContextPolicy, SectionKind};
use super::types::{ContextLoadError, HiddenDuplicateGroup, LoadedContext, SessionSummaryBrief};

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

    LoadedContext {
        memories,
        lessons,
        summaries,
        workstreams,
        errors,
        owner_traces: memory_selection.owner_traces,
        owner_counts: memory_selection.owner_counts,
        diagnostics: memory_selection.diagnostics,
    }
}

struct ContextMemorySelection {
    memories: Vec<Memory>,
    errors: Vec<ContextLoadError>,
    owner_traces: Vec<OwnerTrace>,
    owner_counts: OwnerCounts,
    diagnostics: super::types::ContextDiagnostics,
}

struct ContextMemoryRow {
    memory: Memory,
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

    let excluded_types = policy
        .section(SectionKind::MemoryIndex)
        .map(|section| section.exclude_types.as_slice())
        .unwrap_or(&[]);
    if let Some(implicit_query) = build_implicit_context_query(
        project,
        current_branch,
        commit_messages,
        summaries,
        workstreams,
    ) {
        match query_hybrid_context_memories(
            conn,
            project,
            &implicit_query,
            current_branch,
            excluded_types,
            policy.limits.candidate_fetch_limit as i64,
        ) {
            Ok(retrieved) => {
                for memory in retrieved {
                    if seen_ids.insert(memory.id) {
                        memories.push(memory);
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

    let recent = query_owner_included_memory_rows(
        conn,
        project,
        None,
        current_branch,
        excluded_types,
        policy.limits.candidate_fetch_limit as i64,
    )
    .unwrap_or_else(|e| {
        let message = format!("failed to load recent context memories for {project}: {e}");
        crate::log::error("context", &message);
        errors.push(ContextLoadError::new("memories", message));
        Vec::new()
    });
    for row in recent {
        if seen_ids.insert(row.memory.id) {
            memories.push(row.memory);
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
        errors,
        owner_traces: traces,
        owner_counts,
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

fn sort_memories_by_branch(memories: &mut [Memory], current_branch: Option<&str>) {
    let Some(branch) = current_branch else {
        return;
    };

    memories.sort_by(|left, right| {
        branch_sort_score(left, branch).cmp(&branch_sort_score(right, branch))
    });
}

fn branch_sort_score(memory: &Memory, current_branch: &str) -> u8 {
    match memory.branch.as_deref() {
        Some(branch) if branch == current_branch => 0,
        None => 1,
        Some("main") | Some("master") => 2,
        _ => 3,
    }
}

struct ClusterRepresentative {
    first_index: usize,
    cluster_key: String,
    memory: Memory,
    hidden_ids: Vec<i64>,
}

fn deduplicate_memory_clusters(
    memories: Vec<Memory>,
    current_branch: Option<&str>,
) -> (Vec<Memory>, Vec<HiddenDuplicateGroup>) {
    let mut representatives: HashMap<String, ClusterRepresentative> = HashMap::new();

    for (index, memory) in memories.into_iter().enumerate() {
        let cluster_key = memory_cluster_key(&memory);
        match representatives.get_mut(&cluster_key) {
            Some(representative) => {
                if is_better_cluster_representative(&memory, &representative.memory, current_branch)
                {
                    representative.hidden_ids.push(representative.memory.id);
                    representative.memory = memory;
                } else {
                    representative.hidden_ids.push(memory.id);
                }
            }
            None => {
                representatives.insert(
                    cluster_key.clone(),
                    ClusterRepresentative {
                        first_index: index,
                        cluster_key,
                        memory,
                        hidden_ids: Vec::new(),
                    },
                );
            }
        }
    }

    let mut deduped: Vec<ClusterRepresentative> = representatives.into_values().collect();
    deduped.sort_by_key(|representative| representative.first_index);
    let hidden_groups = deduped
        .iter()
        .filter(|representative| !representative.hidden_ids.is_empty())
        .map(|representative| HiddenDuplicateGroup {
            cluster_key: representative.cluster_key.clone(),
            chosen_id: representative.memory.id,
            hidden_ids: representative.hidden_ids.clone(),
        })
        .collect();
    let memories = deduped
        .into_iter()
        .map(|representative| representative.memory)
        .collect();
    (memories, hidden_groups)
}

fn is_better_cluster_representative(
    candidate: &Memory,
    incumbent: &Memory,
    current_branch: Option<&str>,
) -> bool {
    let candidate_branch_score = current_branch
        .map(|branch| branch_sort_score(candidate, branch))
        .unwrap_or(0);
    let incumbent_branch_score = current_branch
        .map(|branch| branch_sort_score(incumbent, branch))
        .unwrap_or(0);

    candidate_branch_score < incumbent_branch_score
        || (candidate_branch_score == incumbent_branch_score
            && candidate.updated_at_epoch > incumbent.updated_at_epoch)
}

fn limit_self_diagnostic_memories(memories: Vec<Memory>, limit: usize) -> Vec<Memory> {
    let mut retained = Vec::new();
    let mut self_diagnostic_count = 0;

    for memory in memories {
        if is_memory_self_diagnostic(&memory) {
            if self_diagnostic_count >= limit {
                continue;
            }
            self_diagnostic_count += 1;
        }
        retained.push(memory);
    }

    retained
}

fn memory_cluster_key(memory: &Memory) -> String {
    if let Some(topic_key) = stable_topic_key(memory.topic_key.as_deref(), &memory.memory_type) {
        return format!("topic:{topic_key}");
    }

    if let Some(context) = context_prefix(&memory.text) {
        return format!(
            "context:{}:{}",
            memory.memory_type,
            context_cluster_suffix(&normalize_cluster_text(&context))
        );
    }

    format!(
        "title:{}:{}",
        memory.memory_type,
        normalize_cluster_text(&memory.title)
    )
}

fn stable_topic_key<'a>(topic_key: Option<&'a str>, memory_type: &str) -> Option<&'a str> {
    let key = topic_key?.trim();
    if key.is_empty() || looks_generated_topic_key(key, memory_type) {
        return None;
    }
    Some(key)
}

fn looks_generated_topic_key(key: &str, memory_type: &str) -> bool {
    let Some(suffix) = key.strip_prefix(&format!("{memory_type}-")) else {
        return false;
    };
    suffix.len() >= 12 && suffix.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn context_prefix(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix("[Context:")?;
    let end = rest.find(']')?;
    Some(rest[..end].trim().to_string())
}

fn normalize_cluster_text(text: &str) -> String {
    let mut folded = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            folded.extend(ch.to_lowercase());
        } else {
            folded.push(' ');
        }
    }

    let normalized = folded.split_whitespace().collect::<Vec<_>>().join(" ");

    normalized.chars().take(96).collect()
}

fn context_cluster_suffix(normalized_context: &str) -> String {
    let tokens: Vec<&str> = normalized_context.split_whitespace().collect();
    if let Some(reference_key) = reference_cluster_key(&tokens) {
        return reference_key;
    }

    let ascii_tokens: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|token| token.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .filter(|token| !is_context_stop_token(token))
        .take(5)
        .collect();
    if ascii_tokens.len() >= 2 {
        return format!("tokens:{}", ascii_tokens.join("-"));
    }

    normalized_context.chars().take(96).collect()
}

fn reference_cluster_key(tokens: &[&str]) -> Option<String> {
    for window in tokens.windows(2) {
        let label = window[0];
        let value = window[1];
        if matches!(label, "pr" | "pull" | "pullrequest")
            && value.chars().all(|ch| ch.is_ascii_digit())
        {
            return Some(format!("pr:{value}"));
        }
        if matches!(label, "issue" | "issues") && value.chars().all(|ch| ch.is_ascii_digit()) {
            return Some(format!("issue:{value}"));
        }
    }
    None
}

fn is_context_stop_token(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "by"
            | "for"
            | "from"
            | "in"
            | "of"
            | "on"
            | "the"
            | "to"
            | "with"
            | "context"
            | "skills"
            | "skill"
    )
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
