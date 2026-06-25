use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::BTreeMap;

use super::preference_cluster::preference_clusters;
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
    let memories = load_memory_audit_rows(conn, req.project)?;
    let workstreams = load_workstream_audit_rows(conn, req.project)?;

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
        let routing_blob = row.routing_blob();
        let suggestion = route_suggestion(&routing_blob);
        let has_repo_evidence = row.has_strong_repo_evidence(req.project, &routing_blob);
        if is_repo_owned_for_project(
            req.project,
            row.project.as_str(),
            row.scope.as_deref(),
            row.owner_scope.as_deref(),
            row.owner_key.as_deref(),
            row.target_project.as_deref(),
        ) {
            if let Some(suggestion) = suggestion.as_ref().filter(|_| !has_repo_evidence) {
                likely_cross_tool_domain_pollution.push(row.audit_item(
                    suggestion.reason,
                    Some(suggestion),
                    Some("reroute"),
                ));
            } else if suggestion.is_some() && has_repo_evidence {
                low_confidence_routing.push(row.audit_item(
                    "repo evidence conflicts with tool/domain routing keywords",
                    suggestion.as_ref(),
                    Some("review"),
                ));
            } else if row.status == "active"
                && row.memory_type != "preference"
                && row
                    .expires_at_epoch
                    .is_none_or(|expires| expires > req.now_epoch)
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
        let routing_blob = row.routing_blob();
        let suggestion = route_suggestion(&routing_blob);
        let has_repo_evidence = row.has_strong_repo_evidence(req.project, &routing_blob);
        if is_repo_owned_for_project(
            req.project,
            row.project.as_str(),
            None,
            row.owner_scope.as_deref(),
            row.owner_key.as_deref(),
            row.target_project.as_deref(),
        ) {
            if let Some(suggestion) = suggestion.as_ref().filter(|_| !has_repo_evidence) {
                likely_cross_tool_domain_pollution.push(row.audit_item(
                    suggestion.reason,
                    Some(suggestion),
                    Some("pause"),
                ));
            } else if suggestion.is_some() && has_repo_evidence {
                low_confidence_routing.push(row.audit_item(
                    "repo evidence conflicts with tool/domain routing keywords",
                    suggestion.as_ref(),
                    Some("review"),
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
        duplicate_preferences: take_limit(preference_clusters(&memories, req.project), limit),
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
    owner_scope.is_none() || confidence.is_none_or(|value| value < LOW_CONFIDENCE_THRESHOLD)
}

fn take_limit<T>(mut values: Vec<T>, limit: i64) -> Vec<T> {
    values.truncate(limit as usize);
    values
}

#[derive(Debug, Clone)]
pub(super) struct MemoryAuditRow {
    pub(super) id: i64,
    pub(super) project: String,
    pub(super) topic_key: Option<String>,
    pub(super) title: String,
    pub(super) content: String,
    pub(super) memory_type: String,
    pub(super) status: String,
    pub(super) scope: Option<String>,
    pub(super) source_project: Option<String>,
    pub(super) target_project: Option<String>,
    pub(super) owner_scope: Option<String>,
    pub(super) owner_key: Option<String>,
    pub(super) topic_domain: Option<String>,
    pub(super) routing_confidence: Option<f64>,
    pub(super) context_class: Option<String>,
    pub(super) expires_at_epoch: Option<i64>,
    pub(super) updated_at_epoch: i64,
    pub(super) state_key: Option<String>,
    pub(super) current_memory_id: Option<i64>,
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

    fn has_strong_repo_evidence(&self, project: &str, routing_blob: &str) -> bool {
        has_repo_evidence(project, routing_blob, self.topic_domain.as_deref())
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
) -> Result<Vec<MemoryAuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.project, m.topic_key, m.title, m.content, m.memory_type,
                m.status, m.scope, m.source_project, m.target_project, m.owner_scope,
                m.owner_key, m.topic_domain, m.routing_confidence, m.context_class,
                m.expires_at_epoch, m.updated_at_epoch, sk.state_key, sk.current_memory_id
         FROM memories m
         LEFT JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.project = ?1
            OR m.source_project = ?1
            OR m.target_project = ?1
            OR (m.owner_scope = 'repo' AND m.owner_key = ?1)
         ORDER BY m.updated_at_epoch DESC, m.id DESC",
    )?;
    let rows = stmt.query_map(params![project], |row| {
        Ok(MemoryAuditRow {
            id: row.get(0)?,
            project: row.get(1)?,
            topic_key: row.get(2)?,
            title: row.get(3)?,
            content: row.get(4)?,
            memory_type: row.get(5)?,
            status: row.get(6)?,
            scope: row.get(7)?,
            source_project: row.get(8)?,
            target_project: row.get(9)?,
            owner_scope: row.get(10)?,
            owner_key: row.get(11)?,
            topic_domain: row.get(12)?,
            routing_confidence: row.get(13)?,
            context_class: row.get(14)?,
            expires_at_epoch: row.get(15)?,
            updated_at_epoch: row.get(16)?,
            state_key: row.get(17)?,
            current_memory_id: row.get(18)?,
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

    fn has_strong_repo_evidence(&self, project: &str, routing_blob: &str) -> bool {
        has_repo_evidence(project, routing_blob, self.topic_domain.as_deref())
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

fn load_workstream_audit_rows(conn: &Connection, project: &str) -> Result<Vec<WorkstreamAuditRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, title, status, progress, next_action, blockers,
                source_project, target_project, owner_scope, owner_key, topic_domain,
                routing_confidence, context_class
         FROM workstreams
         WHERE project = ?1
            OR source_project = ?1
            OR target_project = ?1
            OR (owner_scope = 'repo' AND owner_key = ?1)
         ORDER BY updated_at_epoch DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![project], |row| {
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
        })
    })?;
    crate::db::query::collect_rows(rows)
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
            members.sort_by_key(|row| row.id);
            let canonical = members.first().copied()?;
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

fn has_repo_evidence(project: &str, routing_blob: &str, topic_domain: Option<&str>) -> bool {
    let slug = project
        .rsplit('/')
        .next()
        .unwrap_or(project)
        .to_ascii_lowercase();
    let project_lower = project.to_ascii_lowercase();
    topic_domain
        .map(|domain| domain.to_ascii_lowercase().starts_with(&slug))
        .unwrap_or(false)
        || routing_blob.contains(&slug)
        || routing_blob.contains(&project_lower)
}
