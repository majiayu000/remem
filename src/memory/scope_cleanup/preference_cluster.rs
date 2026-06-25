use std::collections::{BTreeMap, HashSet};

use crate::memory::preference::consolidation::{
    classify_preference_texts, PreferenceConsolidationKind,
};

use super::audit::{DuplicateCluster, MemoryAuditRow};

pub(super) fn preference_clusters(rows: &[MemoryAuditRow], project: &str) -> Vec<DuplicateCluster> {
    let mut owner_groups: BTreeMap<String, Vec<&MemoryAuditRow>> = BTreeMap::new();
    for row in rows {
        if row.memory_type != "preference" || row.status != "active" {
            continue;
        }
        if !preference_row_is_repo_owned(
            project,
            row.project.as_str(),
            row.scope.as_deref(),
            row.owner_scope.as_deref(),
            row.owner_key.as_deref(),
            row.target_project.as_deref(),
        ) {
            continue;
        }
        owner_groups
            .entry(preference_owner_namespace(row))
            .or_default()
            .push(row);
    }

    let mut clusters = Vec::new();
    for members in owner_groups.values() {
        clusters.extend(semantic_preference_clusters(members));
        clusters.extend(legacy_preference_clusters(members));
    }
    dedup_duplicate_clusters(clusters)
}

fn preference_owner_namespace(row: &MemoryAuditRow) -> String {
    format!(
        "{}:{}:{}",
        row.owner_scope.as_deref().unwrap_or("legacy"),
        row.owner_key.as_deref().unwrap_or(row.project.as_str()),
        row.target_project.as_deref().unwrap_or("")
    )
}

fn semantic_preference_clusters(rows: &[&MemoryAuditRow]) -> Vec<DuplicateCluster> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|row| row.id);
    let mut groups: Vec<Vec<&MemoryAuditRow>> = Vec::new();
    'next_row: for row in sorted {
        for group in &mut groups {
            if group
                .iter()
                .any(|existing| preferences_semantically_overlap(existing, row))
            {
                group.push(row);
                continue 'next_row;
            }
        }
        groups.push(vec![row]);
    }

    groups
        .into_iter()
        .filter_map(|members| {
            let key = preference_cluster_label(&members)
                .unwrap_or_else(|| semantic_preference_cluster_key(&members));
            duplicate_cluster_from_members(
                key,
                members,
                "active preferences are semantically similar and should be represented once",
            )
        })
        .collect()
}

fn preferences_semantically_overlap(existing: &MemoryAuditRow, incoming: &MemoryAuditRow) -> bool {
    classify_preference_texts(existing.id, &existing.content, &incoming.content).is_some_and(
        |matched| {
            matches!(
                matched.kind,
                PreferenceConsolidationKind::SamePreference
                    | PreferenceConsolidationKind::Refinement
            )
        },
    )
}

fn preference_cluster_label(members: &[&MemoryAuditRow]) -> Option<String> {
    let first = members.first()?;
    for key in preference_cluster_keys(first) {
        if key == "unique" || key.starts_with("text:") || key.starts_with("topic:") {
            continue;
        }
        if members
            .iter()
            .all(|row| preference_cluster_keys(row).contains(&key))
        {
            return Some(key);
        }
    }
    None
}

fn semantic_preference_cluster_key(members: &[&MemoryAuditRow]) -> String {
    let canonical = current_member(members)
        .or_else(|| latest_member(members))
        .or_else(|| members.first().copied());
    match canonical {
        Some(row) => format!("semantic:memory:{}", row.id),
        None => "semantic:empty".to_string(),
    }
}

fn legacy_preference_clusters(rows: &[&MemoryAuditRow]) -> Vec<DuplicateCluster> {
    let mut groups: BTreeMap<String, Vec<&MemoryAuditRow>> = BTreeMap::new();
    for row in rows {
        for key in preference_cluster_keys(row) {
            groups.entry(key).or_default().push(row);
        }
    }
    groups
        .into_iter()
        .filter_map(|(key, members)| {
            if key == "unique" {
                return None;
            }
            duplicate_cluster_from_members(
                key,
                members,
                "active preferences overlap and should be represented once",
            )
        })
        .collect()
}

fn dedup_duplicate_clusters(mut clusters: Vec<DuplicateCluster>) -> Vec<DuplicateCluster> {
    let mut seen_member_sets = HashSet::new();
    clusters.sort_by(|left, right| {
        right
            .refs
            .len()
            .cmp(&left.refs.len())
            .then_with(|| left.cluster_key.cmp(&right.cluster_key))
    });
    let mut emitted_sets: Vec<HashSet<i64>> = Vec::new();
    clusters
        .into_iter()
        .filter(|cluster| {
            let member_ids = cluster
                .refs
                .iter()
                .filter_map(|object_ref| object_ref.strip_prefix("memory:"))
                .filter_map(|id| id.parse::<i64>().ok())
                .collect::<Vec<_>>();
            let member_key = member_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            if !seen_member_sets.insert(member_key) {
                return false;
            }
            let member_set = member_ids.into_iter().collect::<HashSet<_>>();
            if emitted_sets
                .iter()
                .any(|emitted| member_set.is_subset(emitted))
            {
                return false;
            }
            emitted_sets.push(member_set);
            true
        })
        .collect()
}

fn duplicate_cluster_from_members(
    cluster_key: String,
    mut members: Vec<&MemoryAuditRow>,
    reason: &str,
) -> Option<DuplicateCluster> {
    if members.len() < 2 {
        return None;
    }
    members.sort_by_key(|row| row.id);
    let canonical = current_member(&members)
        .or_else(|| latest_member(&members))
        .or_else(|| members.first().copied())?;
    let refs = members
        .iter()
        .map(|row| format!("memory:{}", row.id))
        .collect::<Vec<_>>();
    Some(DuplicateCluster {
        cluster_key,
        canonical_ref: format!("memory:{}", canonical.id),
        refs,
        reason: reason.to_string(),
        merged_content: Some(merge_preference_texts(
            &members
                .iter()
                .map(|row| row.content.as_str())
                .collect::<Vec<_>>(),
        )),
    })
}

fn preference_cluster_keys(row: &MemoryAuditRow) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(state_key) = non_empty_cluster_value(row.state_key.as_deref()) {
        keys.push(format!("state:{state_key}"));
    }
    let text = normalize_cluster_text(&format!("{} {}", row.title, row.content));
    if text.contains("ui") && text.contains("critique") {
        keys.push("ui-critique".to_string());
    }
    if text.contains("direct") && text.contains("review") {
        keys.push("direct-review".to_string());
    }
    let content_key = normalize_cluster_text(&row.content);
    if !content_key.is_empty() {
        keys.push(format!("text:{content_key}"));
    }
    if let Some(topic_key) = non_empty_cluster_value(row.topic_key.as_deref()) {
        keys.push(format!("topic:{topic_key}"));
    }
    if keys.is_empty() {
        keys.push("unique".to_string());
    }
    keys
}

fn current_member<'a>(members: &[&'a MemoryAuditRow]) -> Option<&'a MemoryAuditRow> {
    members
        .iter()
        .copied()
        .find(|row| row.current_memory_id == Some(row.id))
}

fn latest_member<'a>(members: &[&'a MemoryAuditRow]) -> Option<&'a MemoryAuditRow> {
    members
        .iter()
        .copied()
        .max_by_key(|row| (row.updated_at_epoch, row.id))
}

fn non_empty_cluster_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn merge_preference_texts(texts: &[&str]) -> String {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    for text in texts {
        let cleaned = text
            .trim()
            .trim_start_matches("Preference:")
            .trim()
            .trim_end_matches('.');
        if cleaned.is_empty() {
            continue;
        }
        let key = normalize_cluster_text(cleaned);
        if seen.insert(key) {
            parts.push(cleaned.to_string());
        }
    }
    format!("Preference: {}.", parts.join("; "))
}

fn normalize_cluster_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if ch.is_alphanumeric() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn preference_row_is_repo_owned(
    project: &str,
    legacy_project: &str,
    scope: Option<&str>,
    owner_scope: Option<&str>,
    owner_key: Option<&str>,
    target_project: Option<&str>,
) -> bool {
    match owner_scope {
        Some("repo") => owner_key == Some(project) || target_project == Some(project),
        Some(_) => false,
        None => legacy_project == project && scope.unwrap_or("project") != "global",
    }
}
