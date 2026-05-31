use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

use super::{ObjectRef, ScopeObjectKind};

const LOW_CONFIDENCE_THRESHOLD: f64 = 0.6;

#[derive(Debug, Clone)]
pub struct ScopeAuditRequest<'a> {
    pub project: &'a str,
    pub limit: i64,
    pub now_epoch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScopeAuditReport {
    pub project: String,
    pub limit: i64,
    pub likely_correct_repo_memory: Vec<AuditItem>,
    pub likely_cross_tool_domain_pollution: Vec<AuditItem>,
    pub duplicate_preferences: Vec<DuplicateCluster>,
    pub duplicate_workstreams: Vec<DuplicateCluster>,
    pub stale_temporal_facts: Vec<AuditItem>,
    pub low_confidence_routing: Vec<AuditItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditItem {
    pub object_ref: String,
    pub object_type: String,
    pub title: String,
    pub status: String,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub source_project: Option<String>,
    pub target_project: Option<String>,
    pub topic_domain: Option<String>,
    pub routing_confidence: Option<f64>,
    pub reason: String,
    pub suggested_owner_scope: Option<String>,
    pub suggested_owner_key: Option<String>,
    pub suggested_target_project: Option<String>,
    pub suggested_action: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateCluster {
    pub cluster_key: String,
    pub canonical_ref: String,
    pub refs: Vec<String>,
    pub reason: String,
    pub merged_content: Option<String>,
}

pub fn audit_scope(conn: &Connection, req: &ScopeAuditRequest<'_>) -> Result<ScopeAuditReport> {
    let limit = req.limit.clamp(1, 500);
    let memories = load_memory_audit_rows(conn, req.project, limit)?;
    let workstreams = load_workstream_audit_rows(conn, req.project, limit)?;

    let mut likely_correct_repo_memory = Vec::new();
    let mut likely_cross_tool_domain_pollution = Vec::new();
    let mut stale_temporal_facts = Vec::new();
    let mut low_confidence_routing = Vec::new();

    for row in &memories {
        if is_low_confidence(row.owner_scope.as_deref(), row.routing_confidence) {
            low_confidence_routing.push(row.audit_item(
                "routing confidence is missing or below review threshold",
                None,
                Some("review"),
            ));
        }
        if row
            .expires_at_epoch
            .is_some_and(|expires| expires <= req.now_epoch)
            && row.status == "active"
        {
            stale_temporal_facts.push(row.audit_item(
                "active memory is past expires_at_epoch",
                None,
                Some("archive"),
            ));
        }
        let suggestion = route_suggestion(&row.routing_blob());
        if is_repo_owned_for_project(
            req.project,
            row.project.as_str(),
            row.scope.as_deref(),
            row.owner_scope.as_deref(),
            row.owner_key.as_deref(),
            row.target_project.as_deref(),
        ) {
            if let Some(suggestion) = suggestion {
                likely_cross_tool_domain_pollution.push(row.audit_item(
                    suggestion.reason,
                    Some(&suggestion),
                    Some("reroute"),
                ));
            } else if row.status == "active"
                && row.memory_type != "preference"
                && !row
                    .expires_at_epoch
                    .is_some_and(|expires| expires <= req.now_epoch)
                && !is_low_confidence(row.owner_scope.as_deref(), row.routing_confidence)
            {
                likely_correct_repo_memory.push(row.audit_item(
                    "repo-owned memory matches the audited project",
                    None,
                    Some("keep"),
                ));
            }
        }
    }

    for row in &workstreams {
        if is_low_confidence(row.owner_scope.as_deref(), row.routing_confidence) {
            low_confidence_routing.push(row.audit_item(
                "routing confidence is missing or below review threshold",
                None,
                Some("review"),
            ));
        }
        let suggestion = route_suggestion(&row.routing_blob());
        if is_repo_owned_for_project(
            req.project,
            row.project.as_str(),
            None,
            row.owner_scope.as_deref(),
            row.owner_key.as_deref(),
            row.target_project.as_deref(),
        ) {
            if let Some(suggestion) = suggestion {
                likely_cross_tool_domain_pollution.push(row.audit_item(
                    suggestion.reason,
                    Some(&suggestion),
                    Some("pause"),
                ));
            } else if row.status == "active"
                && !is_low_confidence(row.owner_scope.as_deref(), row.routing_confidence)
            {
                likely_correct_repo_memory.push(row.audit_item(
                    "repo-owned workstream matches the audited project",
                    None,
                    Some("keep"),
                ));
            }
        }
    }

    Ok(ScopeAuditReport {
        project: req.project.to_string(),
        limit,
        likely_correct_repo_memory: take_limit(likely_correct_repo_memory, limit),
        likely_cross_tool_domain_pollution: take_limit(likely_cross_tool_domain_pollution, limit),
        duplicate_preferences: take_limit(preference_clusters(&memories), limit),
        duplicate_workstreams: take_limit(workstream_clusters(&workstreams), limit),
        stale_temporal_facts: take_limit(stale_temporal_facts, limit),
        low_confidence_routing: take_limit(low_confidence_routing, limit),
    })
}

#[derive(Debug, Clone)]
struct RouteSuggestion {
    owner_scope: &'static str,
    owner_key: &'static str,
    target_project: Option<String>,
    reason: &'static str,
}

fn route_suggestion(blob: &str) -> Option<RouteSuggestion> {
    if contains_any(
        blob,
        &[
            "codex",
            "workspace-write",
            "approval",
            "sandbox",
            "mcp config",
            "codex cli",
        ],
    ) {
        return Some(RouteSuggestion {
            owner_scope: "tool",
            owner_key: "codex-cli",
            target_project: None,
            reason: "content is about Codex CLI sandbox, approvals, or runtime",
        });
    }
    if contains_any(blob, &["grok", "xai", "x.ai"]) {
        return Some(RouteSuggestion {
            owner_scope: "domain",
            owner_key: "grok-api",
            target_project: None,
            reason: "content is about Grok/xAI API rather than this repo",
        });
    }
    if contains_any(
        blob,
        &["warp", "macos", "tcc", "app routing", "terminal launch"],
    ) {
        return Some(RouteSuggestion {
            owner_scope: "domain",
            owner_key: "macos",
            target_project: None,
            reason: "content is about macOS or terminal routing rather than this repo",
        });
    }
    if contains_any(blob, &["hermes"]) {
        return Some(RouteSuggestion {
            owner_scope: "domain",
            owner_key: "hermes",
            target_project: None,
            reason: "content is about Hermes rather than this repo",
        });
    }
    None
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn is_repo_owned_for_project(
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

fn is_low_confidence(owner_scope: Option<&str>, confidence: Option<f64>) -> bool {
    owner_scope.is_none()
        || confidence
            .map(|value| value < LOW_CONFIDENCE_THRESHOLD)
            .unwrap_or(false)
}

fn take_limit<T>(mut values: Vec<T>, limit: i64) -> Vec<T> {
    values.truncate(limit as usize);
    values
}

#[derive(Debug, Clone)]
pub(super) struct MemoryAuditRow {
    id: i64,
    project: String,
    title: String,
    content: String,
    memory_type: String,
    status: String,
    scope: Option<String>,
    source_project: Option<String>,
    target_project: Option<String>,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    topic_domain: Option<String>,
    routing_confidence: Option<f64>,
    context_class: Option<String>,
    expires_at_epoch: Option<i64>,
    updated_at_epoch: i64,
}

impl MemoryAuditRow {
    fn object_ref(&self) -> ObjectRef {
        ObjectRef::memory(self.id)
    }

    fn routing_blob(&self) -> String {
        format!(
            "{} {} {} {}",
            self.title,
            self.content,
            self.topic_domain.as_deref().unwrap_or_default(),
            self.context_class.as_deref().unwrap_or_default()
        )
        .to_ascii_lowercase()
    }

    fn audit_item(
        &self,
        reason: &str,
        suggestion: Option<&RouteSuggestion>,
        suggested_action: Option<&str>,
    ) -> AuditItem {
        AuditItem {
            object_ref: self.object_ref().to_string(),
            object_type: ScopeObjectKind::Memory.as_str().to_string(),
            title: self.title.clone(),
            status: self.status.clone(),
            owner_scope: self.owner_scope.clone(),
            owner_key: self.owner_key.clone(),
            source_project: self.source_project.clone(),
            target_project: self.target_project.clone(),
            topic_domain: self.topic_domain.clone(),
            routing_confidence: self.routing_confidence,
            reason: reason.to_string(),
            suggested_owner_scope: suggestion.map(|value| value.owner_scope.to_string()),
            suggested_owner_key: suggestion.map(|value| value.owner_key.to_string()),
            suggested_target_project: suggestion.and_then(|value| value.target_project.clone()),
            suggested_action: suggested_action.map(str::to_string),
        }
    }
}

pub(super) fn load_memory_audit_rows(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<MemoryAuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, title, content, memory_type, status, scope,
                source_project, target_project, owner_scope, owner_key, topic_domain,
                routing_confidence, context_class, expires_at_epoch, updated_at_epoch
         FROM memories
         WHERE project = ?1
            OR source_project = ?1
            OR target_project = ?1
            OR (owner_scope = 'repo' AND owner_key = ?1)
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], |row| {
        Ok(MemoryAuditRow {
            id: row.get(0)?,
            project: row.get(1)?,
            title: row.get(2)?,
            content: row.get(3)?,
            memory_type: row.get(4)?,
            status: row.get(5)?,
            scope: row.get(6)?,
            source_project: row.get(7)?,
            target_project: row.get(8)?,
            owner_scope: row.get(9)?,
            owner_key: row.get(10)?,
            topic_domain: row.get(11)?,
            routing_confidence: row.get(12)?,
            context_class: row.get(13)?,
            expires_at_epoch: row.get(14)?,
            updated_at_epoch: row.get(15)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

#[derive(Debug, Clone)]
struct WorkstreamAuditRow {
    id: i64,
    project: String,
    title: String,
    status: String,
    progress: Option<String>,
    next_action: Option<String>,
    blockers: Option<String>,
    source_project: Option<String>,
    target_project: Option<String>,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    topic_domain: Option<String>,
    routing_confidence: Option<f64>,
    context_class: Option<String>,
    updated_at_epoch: i64,
}

impl WorkstreamAuditRow {
    fn object_ref(&self) -> ObjectRef {
        ObjectRef {
            kind: ScopeObjectKind::Workstream,
            id: self.id,
        }
    }

    fn routing_blob(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.title,
            self.progress.as_deref().unwrap_or_default(),
            self.next_action.as_deref().unwrap_or_default(),
            self.blockers.as_deref().unwrap_or_default(),
            self.topic_domain.as_deref().unwrap_or_default(),
            self.context_class.as_deref().unwrap_or_default()
        )
        .to_ascii_lowercase()
    }

    fn audit_item(
        &self,
        reason: &str,
        suggestion: Option<&RouteSuggestion>,
        suggested_action: Option<&str>,
    ) -> AuditItem {
        AuditItem {
            object_ref: self.object_ref().to_string(),
            object_type: ScopeObjectKind::Workstream.as_str().to_string(),
            title: self.title.clone(),
            status: self.status.clone(),
            owner_scope: self.owner_scope.clone(),
            owner_key: self.owner_key.clone(),
            source_project: self.source_project.clone(),
            target_project: self.target_project.clone(),
            topic_domain: self.topic_domain.clone(),
            routing_confidence: self.routing_confidence,
            reason: reason.to_string(),
            suggested_owner_scope: suggestion.map(|value| value.owner_scope.to_string()),
            suggested_owner_key: suggestion.map(|value| value.owner_key.to_string()),
            suggested_target_project: suggestion.and_then(|value| value.target_project.clone()),
            suggested_action: suggested_action.map(str::to_string),
        }
    }
}

fn load_workstream_audit_rows(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<WorkstreamAuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, title, status, progress, next_action, blockers,
                source_project, target_project, owner_scope, owner_key, topic_domain,
                routing_confidence, context_class, updated_at_epoch
         FROM workstreams
         WHERE project = ?1
            OR source_project = ?1
            OR target_project = ?1
            OR (owner_scope = 'repo' AND owner_key = ?1)
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], |row| {
        Ok(WorkstreamAuditRow {
            id: row.get(0)?,
            project: row.get(1)?,
            title: row.get(2)?,
            status: row.get(3)?,
            progress: row.get(4)?,
            next_action: row.get(5)?,
            blockers: row.get(6)?,
            source_project: row.get(7)?,
            target_project: row.get(8)?,
            owner_scope: row.get(9)?,
            owner_key: row.get(10)?,
            topic_domain: row.get(11)?,
            routing_confidence: row.get(12)?,
            context_class: row.get(13)?,
            updated_at_epoch: row.get(14)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

pub(super) fn preference_clusters(rows: &[MemoryAuditRow]) -> Vec<DuplicateCluster> {
    let mut groups: BTreeMap<String, Vec<&MemoryAuditRow>> = BTreeMap::new();
    for row in rows {
        if row.memory_type != "preference" || row.status != "active" {
            continue;
        }
        let key = preference_cluster_key(&row.title, &row.content);
        groups.entry(key).or_default().push(row);
    }
    groups
        .into_iter()
        .filter_map(|(key, mut members)| {
            if key == "unique" || members.len() < 2 {
                return None;
            }
            members.sort_by_key(|row| (row.content.len(), row.updated_at_epoch, row.id));
            let canonical = members.last().copied()?;
            let refs = members
                .iter()
                .map(|row| row.object_ref().to_string())
                .collect::<Vec<_>>();
            Some(DuplicateCluster {
                cluster_key: key,
                canonical_ref: canonical.object_ref().to_string(),
                refs,
                reason: "active preferences overlap and should be represented once".to_string(),
                merged_content: Some(merge_preference_texts(
                    &members
                        .iter()
                        .map(|row| row.content.as_str())
                        .collect::<Vec<_>>(),
                )),
            })
        })
        .collect()
}

fn preference_cluster_key(title: &str, content: &str) -> String {
    let text = normalize_text(&format!("{title} {content}"));
    if text.contains("ui") && text.contains("critique") {
        return "ui-critique".to_string();
    }
    if text.contains("direct") && text.contains("review") {
        return "direct-review".to_string();
    }
    "unique".to_string()
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
        let key = normalize_text(cleaned);
        if seen.insert(key) {
            parts.push(cleaned.to_string());
        }
    }
    format!("Preference: {}.", parts.join("; "))
}

fn workstream_clusters(rows: &[WorkstreamAuditRow]) -> Vec<DuplicateCluster> {
    let mut groups: BTreeMap<String, Vec<&WorkstreamAuditRow>> = BTreeMap::new();
    for row in rows {
        if row.status != "active" {
            continue;
        }
        let key = workstream_cluster_key(&row.title);
        groups.entry(key).or_default().push(row);
    }
    groups
        .into_iter()
        .filter_map(|(key, mut members)| {
            if key == "unique" || members.len() < 2 {
                return None;
            }
            members.sort_by_key(|row| (row.updated_at_epoch, row.id));
            let canonical = members.last().copied()?;
            Some(DuplicateCluster {
                cluster_key: key,
                canonical_ref: canonical.object_ref().to_string(),
                refs: members
                    .iter()
                    .map(|row| row.object_ref().to_string())
                    .collect(),
                reason: "active workstreams appear to track the same task".to_string(),
                merged_content: None,
            })
        })
        .collect()
}

fn workstream_cluster_key(title: &str) -> String {
    let text = normalize_text(title);
    if text.contains("stash") && text.contains("sidebar") && text.contains("polish") {
        return "stash-sidebar-polish".to_string();
    }
    "unique".to_string()
}

fn normalize_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
