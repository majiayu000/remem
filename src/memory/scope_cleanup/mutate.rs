use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::{ObjectRef, ScopeObjectKind};

const DEFAULT_ROUTING_CONFIDENCE: f64 = 1.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetProjectUpdate {
    Preserve,
    Clear,
    Set(String),
}

#[derive(Debug, Clone)]
pub struct RerouteRequest<'a> {
    pub refs: &'a [ObjectRef],
    pub owner_scope: &'a str,
    pub owner_key: &'a str,
    pub target_project: TargetProjectUpdate,
    pub topic_domain: Option<&'a str>,
    pub context_class: Option<&'a str>,
    pub routing_confidence: Option<f64>,
    pub reason: Option<&'a str>,
    pub dry_run: bool,
    pub confirm: bool,
}

#[derive(Debug, Clone)]
pub struct ArchiveRequest<'a> {
    pub refs: &'a [ObjectRef],
    pub reason: Option<&'a str>,
    pub dry_run: bool,
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScopeMutationResult {
    pub dry_run: bool,
    pub action: String,
    pub affected: Vec<ObjectMutation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectMutation {
    pub object_ref: String,
    pub title: String,
    pub previous_status: String,
    pub new_status: String,
    pub previous_owner: OwnerSnapshot,
    pub new_owner: OwnerSnapshot,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OwnerSnapshot {
    pub source_project: Option<String>,
    pub target_project: Option<String>,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub topic_domain: Option<String>,
    pub routing_confidence: Option<f64>,
    pub routing_reason: Option<String>,
    pub context_class: Option<String>,
}

pub fn reroute_objects(conn: &Connection, req: &RerouteRequest<'_>) -> Result<ScopeMutationResult> {
    ensure_refs(req.refs)?;
    let (owner_scope, owner_key) = normalize_owner(req.owner_scope, req.owner_key)?;
    let reason = normalized_reason(req.reason);
    let dry_run = req.dry_run || !req.confirm;
    let tx = conn.unchecked_transaction()?;
    let targets = load_targets(&tx, req.refs)?;
    let now = chrono::Utc::now().timestamp();
    let mut affected = Vec::with_capacity(targets.len());

    for target in targets {
        let new_owner = OwnerSnapshot {
            source_project: target
                .owner
                .source_project
                .clone()
                .or_else(|| target.project.clone()),
            target_project: target_project_after(
                &target.owner.target_project,
                &req.target_project,
            )?,
            owner_scope: Some(owner_scope.clone()),
            owner_key: Some(owner_key.clone()),
            topic_domain: req
                .topic_domain
                .map(str::to_string)
                .or_else(|| target.owner.topic_domain.clone()),
            routing_confidence: Some(req.routing_confidence.unwrap_or(DEFAULT_ROUTING_CONFIDENCE)),
            routing_reason: reason
                .clone()
                .or_else(|| Some("manual scope cleanup reroute".to_string())),
            context_class: req
                .context_class
                .map(str::to_string)
                .or_else(|| default_context_class(&owner_scope).map(str::to_string))
                .or_else(|| target.owner.context_class.clone()),
        };
        affected.push(ObjectMutation {
            object_ref: target.object_ref.to_string(),
            title: target.title.clone(),
            previous_status: target.status.clone(),
            new_status: target.status.clone(),
            previous_owner: target.owner.clone(),
            new_owner: new_owner.clone(),
        });
        if dry_run {
            continue;
        }
        update_owner(&tx, target.object_ref, &new_owner, now)?;
        insert_scope_cleanup_event(
            &tx,
            "reroute",
            &target,
            &target.status,
            &new_owner,
            reason.as_deref(),
            now,
        )?;
    }
    tx.commit()?;
    Ok(ScopeMutationResult {
        dry_run,
        action: "reroute".to_string(),
        affected,
    })
}

pub fn archive_objects(conn: &Connection, req: &ArchiveRequest<'_>) -> Result<ScopeMutationResult> {
    ensure_refs(req.refs)?;
    let reason = normalized_reason(req.reason);
    let dry_run = req.dry_run || !req.confirm;
    let tx = conn.unchecked_transaction()?;
    let targets = load_targets(&tx, req.refs)?;
    let now = chrono::Utc::now().timestamp();
    let mut affected = Vec::with_capacity(targets.len());

    for target in targets {
        let new_status = archive_status(target.object_ref.kind);
        affected.push(ObjectMutation {
            object_ref: target.object_ref.to_string(),
            title: target.title.clone(),
            previous_status: target.status.clone(),
            new_status: new_status.to_string(),
            previous_owner: target.owner.clone(),
            new_owner: target.owner.clone(),
        });
        if dry_run {
            continue;
        }
        update_status(&tx, target.object_ref, new_status, now)?;
        insert_scope_cleanup_event(
            &tx,
            "archive",
            &target,
            new_status,
            &target.owner,
            reason.as_deref(),
            now,
        )?;
    }
    tx.commit()?;
    Ok(ScopeMutationResult {
        dry_run,
        action: "archive".to_string(),
        affected,
    })
}

fn ensure_refs(refs: &[ObjectRef]) -> Result<()> {
    if refs.is_empty() {
        bail!("at least one object ref is required");
    }
    Ok(())
}

fn normalize_owner(owner_scope: &str, owner_key: &str) -> Result<(String, String)> {
    let owner_scope = owner_scope.trim();
    let owner_key = owner_key.trim();
    if owner_key.is_empty() {
        bail!("owner-key must not be empty");
    }
    if !matches!(
        owner_scope,
        "user" | "workspace" | "repo" | "tool" | "domain" | "workstream" | "session"
    ) {
        bail!("unsupported owner-scope: {owner_scope}");
    }
    Ok((owner_scope.to_string(), owner_key.to_string()))
}

pub(super) fn normalized_reason(reason: Option<&str>) -> Option<String> {
    reason
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn default_context_class(owner_scope: &str) -> Option<&'static str> {
    match owner_scope {
        "repo" | "user" | "workspace" => Some("startup_core"),
        "tool" | "domain" => Some("search_only"),
        "workstream" => Some("task_relevant"),
        "session" => Some("never_inject"),
        _ => None,
    }
}

fn archive_status(kind: ScopeObjectKind) -> &'static str {
    match kind {
        ScopeObjectKind::Memory => "archived",
        ScopeObjectKind::Candidate => "discarded",
        ScopeObjectKind::Workstream => "paused",
        ScopeObjectKind::SessionSummary => "never_inject",
    }
}

fn target_project_after(
    previous: &Option<String>,
    update: &TargetProjectUpdate,
) -> Result<Option<String>> {
    match update {
        TargetProjectUpdate::Preserve => Ok(previous.clone()),
        TargetProjectUpdate::Clear => Ok(None),
        TargetProjectUpdate::Set(value) => {
            let value = value.trim();
            if value.is_empty() {
                bail!("target-project must not be empty");
            }
            Ok(Some(value.to_string()))
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct MutationTarget {
    pub object_ref: ObjectRef,
    pub project: Option<String>,
    pub title: String,
    pub status: String,
    pub owner: OwnerSnapshot,
}

fn load_targets(conn: &Connection, refs: &[ObjectRef]) -> Result<Vec<MutationTarget>> {
    refs.iter()
        .copied()
        .map(|object_ref| load_target(conn, object_ref))
        .collect()
}

pub(super) fn load_target(conn: &Connection, object_ref: ObjectRef) -> Result<MutationTarget> {
    match object_ref.kind {
        ScopeObjectKind::Memory => conn
            .query_row(
                "SELECT project, title, status, source_project, target_project,
                        owner_scope, owner_key, topic_domain, routing_confidence,
                        routing_reason, context_class
                 FROM memories WHERE id = ?1",
                params![object_ref.id],
                |row| {
                    Ok(MutationTarget {
                        object_ref,
                        project: row.get(0)?,
                        title: row.get(1)?,
                        status: row.get(2)?,
                        owner: owner_snapshot_from_row(row, 3)?,
                    })
                },
            )
            .optional()?,
        ScopeObjectKind::Workstream => conn
            .query_row(
                "SELECT project, title, status, source_project, target_project,
                        owner_scope, owner_key, topic_domain, routing_confidence,
                        routing_reason, context_class
                 FROM workstreams WHERE id = ?1",
                params![object_ref.id],
                |row| {
                    Ok(MutationTarget {
                        object_ref,
                        project: row.get(0)?,
                        title: row.get(1)?,
                        status: row.get(2)?,
                        owner: owner_snapshot_from_row(row, 3)?,
                    })
                },
            )
            .optional()?,
        ScopeObjectKind::Candidate => conn
            .query_row(
                "SELECT p.project_path, c.topic_key, c.review_status, c.source_project,
                        c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                        c.routing_confidence, c.routing_reason, c.context_class
                 FROM memory_candidates c
                 LEFT JOIN projects p ON p.id = c.project_id
                 WHERE c.id = ?1",
                params![object_ref.id],
                |row| {
                    Ok(MutationTarget {
                        object_ref,
                        project: row.get(0)?,
                        title: row.get(1)?,
                        status: row.get(2)?,
                        owner: owner_snapshot_from_row(row, 3)?,
                    })
                },
            )
            .optional()?,
        ScopeObjectKind::SessionSummary => conn
            .query_row(
                "SELECT project, COALESCE(request, memory_session_id, 'session summary'),
                        COALESCE(context_class, 'search_only'), source_project,
                        target_project, owner_scope, owner_key, topic_domain,
                        routing_confidence, routing_reason, context_class
                 FROM session_summaries WHERE id = ?1",
                params![object_ref.id],
                |row| {
                    Ok(MutationTarget {
                        object_ref,
                        project: row.get(0)?,
                        title: row.get(1)?,
                        status: row.get(2)?,
                        owner: owner_snapshot_from_row(row, 3)?,
                    })
                },
            )
            .optional()?,
    }
    .ok_or_else(|| anyhow!("{} not found", object_ref))
}

fn owner_snapshot_from_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<OwnerSnapshot> {
    Ok(OwnerSnapshot {
        source_project: row.get(offset)?,
        target_project: row.get(offset + 1)?,
        owner_scope: row.get(offset + 2)?,
        owner_key: row.get(offset + 3)?,
        topic_domain: row.get(offset + 4)?,
        routing_confidence: row.get(offset + 5)?,
        routing_reason: row.get(offset + 6)?,
        context_class: row.get(offset + 7)?,
    })
}

fn update_owner(
    conn: &Connection,
    object_ref: ObjectRef,
    owner: &OwnerSnapshot,
    now: i64,
) -> Result<()> {
    let updated = match object_ref.kind {
        ScopeObjectKind::Memory => conn.execute(
            "UPDATE memories
             SET source_project = ?1, target_project = ?2, owner_scope = ?3,
                 owner_key = ?4, topic_domain = ?5, routing_confidence = ?6,
                 routing_reason = ?7, context_class = ?8, updated_at_epoch = ?9
             WHERE id = ?10",
            params![
                owner.source_project.as_deref(),
                owner.target_project.as_deref(),
                owner.owner_scope.as_deref(),
                owner.owner_key.as_deref(),
                owner.topic_domain.as_deref(),
                owner.routing_confidence,
                owner.routing_reason.as_deref(),
                owner.context_class.as_deref(),
                now,
                object_ref.id
            ],
        )?,
        ScopeObjectKind::Workstream => conn.execute(
            "UPDATE workstreams
             SET source_project = ?1, target_project = ?2, owner_scope = ?3,
                 owner_key = ?4, topic_domain = ?5, routing_confidence = ?6,
                 routing_reason = ?7, context_class = ?8, updated_at_epoch = ?9
             WHERE id = ?10",
            params![
                owner.source_project.as_deref(),
                owner.target_project.as_deref(),
                owner.owner_scope.as_deref(),
                owner.owner_key.as_deref(),
                owner.topic_domain.as_deref(),
                owner.routing_confidence,
                owner.routing_reason.as_deref(),
                owner.context_class.as_deref(),
                now,
                object_ref.id
            ],
        )?,
        ScopeObjectKind::Candidate => conn.execute(
            "UPDATE memory_candidates
             SET source_project = ?1, target_project = ?2, owner_scope = ?3,
                 owner_key = ?4, topic_domain = ?5, routing_confidence = ?6,
                 routing_reason = ?7, context_class = ?8, updated_at_epoch = ?9
             WHERE id = ?10",
            params![
                owner.source_project.as_deref(),
                owner.target_project.as_deref(),
                owner.owner_scope.as_deref(),
                owner.owner_key.as_deref(),
                owner.topic_domain.as_deref(),
                owner.routing_confidence,
                owner.routing_reason.as_deref(),
                owner.context_class.as_deref(),
                now,
                object_ref.id
            ],
        )?,
        ScopeObjectKind::SessionSummary => conn.execute(
            "UPDATE session_summaries
             SET source_project = ?1, target_project = ?2, owner_scope = ?3,
                 owner_key = ?4, topic_domain = ?5, routing_confidence = ?6,
                 routing_reason = ?7, context_class = ?8
             WHERE id = ?9",
            params![
                owner.source_project.as_deref(),
                owner.target_project.as_deref(),
                owner.owner_scope.as_deref(),
                owner.owner_key.as_deref(),
                owner.topic_domain.as_deref(),
                owner.routing_confidence,
                owner.routing_reason.as_deref(),
                owner.context_class.as_deref(),
                object_ref.id
            ],
        )?,
    };
    if updated != 1 {
        bail!("failed to update owner for {}", object_ref);
    }
    Ok(())
}

fn update_status(
    conn: &Connection,
    object_ref: ObjectRef,
    new_status: &str,
    now: i64,
) -> Result<()> {
    let updated = match object_ref.kind {
        ScopeObjectKind::Memory => conn.execute(
            "UPDATE memories SET status = ?1, updated_at_epoch = ?2 WHERE id = ?3",
            params![new_status, now, object_ref.id],
        )?,
        ScopeObjectKind::Workstream => conn.execute(
            "UPDATE workstreams SET status = ?1, updated_at_epoch = ?2 WHERE id = ?3",
            params![new_status, now, object_ref.id],
        )?,
        ScopeObjectKind::Candidate => conn.execute(
            "UPDATE memory_candidates SET review_status = ?1, updated_at_epoch = ?2 WHERE id = ?3",
            params![new_status, now, object_ref.id],
        )?,
        ScopeObjectKind::SessionSummary => conn.execute(
            "UPDATE session_summaries SET context_class = ?1 WHERE id = ?2",
            params![new_status, object_ref.id],
        )?,
    };
    if updated != 1 {
        bail!("failed to update status for {}", object_ref);
    }
    Ok(())
}

pub(super) fn insert_scope_cleanup_event(
    conn: &Connection,
    action: &str,
    target: &MutationTarget,
    new_status: &str,
    new_owner: &OwnerSnapshot,
    reason: Option<&str>,
    now: i64,
) -> Result<()> {
    let project = target
        .owner
        .source_project
        .as_deref()
        .or(target.project.as_deref())
        .unwrap_or("<unknown>");
    let detail = serde_json::json!({
        "action": action,
        "object_ref": target.object_ref.to_string(),
        "title": target.title,
        "previous_status": target.status,
        "new_status": new_status,
        "previous_owner": &target.owner,
        "new_owner": new_owner,
        "reason": reason,
    })
    .to_string();
    let summary = format!(
        "{} {}: {} -> {}{}",
        action,
        target.object_ref,
        target.status,
        new_status,
        reason
            .map(|value| format!(" ({value})"))
            .unwrap_or_default()
    );
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, detail, files, exit_code, created_at_epoch)
         VALUES ('scope-cleanup', ?1, 'scope_cleanup', ?2, ?3, NULL, NULL, ?4)",
        params![project, summary, detail, now],
    )?;
    Ok(())
}
