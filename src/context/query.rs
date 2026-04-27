use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::memory_traits::{is_memory_self_diagnostic, is_self_diagnostic_text};
use super::types::{LoadedContext, SessionSummaryBrief};

const CONTEXT_MEMORY_LIMIT: usize = 50;
const RECENT_MEMORY_FETCH_LIMIT: i64 = 100;
const BASENAME_SEARCH_LIMIT: i64 = 20;
const MAX_SELF_DIAGNOSTIC_MEMORIES: usize = 2;
const SUMMARY_FETCH_BATCH_SIZE: usize = 25;
const SUMMARY_MAX_SCAN: usize = 200;
const STALE_DESIGN_SUMMARY_DAYS: i64 = 7;

pub(super) fn load_context_data(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
) -> LoadedContext {
    let mut memories = load_project_memories(conn, project, current_branch);
    sort_memories_by_branch(&mut memories, current_branch);

    let summaries = query_recent_summaries(conn, project, 5).unwrap_or_default();
    let workstreams =
        crate::workstream::query_active_workstreams(conn, project).unwrap_or_default();

    LoadedContext {
        memories,
        summaries,
        workstreams,
    }
}

fn load_project_memories(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
) -> Vec<Memory> {
    let mut memories = Vec::new();
    let mut seen_ids = HashSet::new();

    let recent =
        memory::get_recent_memories(conn, project, RECENT_MEMORY_FETCH_LIMIT).unwrap_or_default();
    for memory in recent {
        if seen_ids.insert(memory.id) {
            memories.push(memory);
        }
    }

    let project_query = project.rsplit('/').next().unwrap_or(project);
    if let Ok(searched) = crate::search::search(
        conn,
        Some(project_query),
        Some(project),
        None,
        BASENAME_SEARCH_LIMIT,
        0,
        false,
    ) {
        for memory in searched {
            if seen_ids.insert(memory.id) {
                memories.push(memory);
            }
        }
    }

    let mut selected =
        limit_self_diagnostic_memories(deduplicate_memory_clusters(memories, current_branch));
    sort_memories_by_branch(&mut selected, current_branch);
    selected.into_iter().take(CONTEXT_MEMORY_LIMIT).collect()
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
    memory: Memory,
}

fn deduplicate_memory_clusters(memories: Vec<Memory>, current_branch: Option<&str>) -> Vec<Memory> {
    let mut representatives: HashMap<String, ClusterRepresentative> = HashMap::new();

    for (index, memory) in memories.into_iter().enumerate() {
        let cluster_key = memory_cluster_key(&memory);
        match representatives.get_mut(&cluster_key) {
            Some(representative) => {
                if is_better_cluster_representative(&memory, &representative.memory, current_branch)
                {
                    representative.memory = memory;
                }
            }
            None => {
                representatives.insert(
                    cluster_key,
                    ClusterRepresentative {
                        first_index: index,
                        memory,
                    },
                );
            }
        }
    }

    let mut deduped: Vec<ClusterRepresentative> = representatives.into_values().collect();
    deduped.sort_by_key(|representative| representative.first_index);
    deduped
        .into_iter()
        .map(|representative| representative.memory)
        .collect()
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

fn limit_self_diagnostic_memories(memories: Vec<Memory>) -> Vec<Memory> {
    let mut retained = Vec::new();
    let mut self_diagnostic_count = 0;

    for memory in memories {
        if is_memory_self_diagnostic(&memory) {
            if self_diagnostic_count >= MAX_SELF_DIAGNOSTIC_MEMORIES {
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

    let now_epoch = chrono::Utc::now().timestamp();
    let scan_limit = SUMMARY_MAX_SCAN.max(limit);
    let mut selected = Vec::new();
    let mut fallback = Vec::new();
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

            if !seen_clusters.insert(summary_cluster_key(&summary)) {
                continue;
            }

            if is_stale_design_prototype_summary(&summary, now_epoch) {
                fallback.push(summary);
                continue;
            }

            selected.push(summary);
            if selected.len() >= limit {
                break;
            }
        }

        offset += fetch_limit;
    }

    if selected.is_empty() {
        selected.extend(fallback.into_iter().take(limit));
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
         WHERE project = ?1 AND request IS NOT NULL AND request != '' \
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
    crate::db_query::collect_rows(rows)
}

fn is_session_summary_self_diagnostic(summary: &SessionSummaryBrief) -> bool {
    let haystack = format!(
        "{} {}",
        summary.request,
        summary.completed.as_deref().unwrap_or_default()
    );
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
