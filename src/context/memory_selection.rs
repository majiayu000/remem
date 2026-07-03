use std::collections::HashMap;

use crate::memory::Memory;

use super::memory_traits::is_memory_self_diagnostic;
use super::types::HiddenDuplicateGroup;

pub(super) fn sort_memories_by_branch(memories: &mut [Memory], current_branch: Option<&str>) {
    let Some(branch) = current_branch else {
        return;
    };

    memories.sort_by(|left, right| {
        branch_sort_score(left, branch)
            .cmp(&branch_sort_score(right, branch))
            .then_with(|| right.updated_at_epoch.cmp(&left.updated_at_epoch))
            .then_with(|| left.id.cmp(&right.id))
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

pub(super) fn deduplicate_memory_clusters(
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
        || (candidate_branch_score == incumbent_branch_score
            && candidate.updated_at_epoch == incumbent.updated_at_epoch
            && candidate.id < incumbent.id)
}

pub(super) fn limit_self_diagnostic_memories(memories: Vec<Memory>, limit: usize) -> Vec<Memory> {
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

pub(super) fn normalize_cluster_text(text: &str) -> String {
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

pub(super) fn context_cluster_suffix(normalized_context: &str) -> String {
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

pub(super) fn reference_cluster_key(tokens: &[&str]) -> Option<String> {
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
