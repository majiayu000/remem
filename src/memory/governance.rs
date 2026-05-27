use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryGovernanceAction {
    Delete,
    Reject,
    MarkStale,
}

impl MemoryGovernanceAction {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_lowercase().as_str() {
            "delete" | "deleted" => Ok(Self::Delete),
            "reject" | "rejected" => Ok(Self::Reject),
            "stale" | "mark_stale" | "mark-stale" | "invalidate" => Ok(Self::MarkStale),
            other => bail!("unsupported memory governance action: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Reject => "reject",
            Self::MarkStale => "stale",
        }
    }

    pub fn target_status(self) -> &'static str {
        match self {
            Self::Delete => "deleted",
            Self::Reject => "rejected",
            Self::MarkStale => "stale",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GovernMemoryRequest<'a> {
    pub project: &'a str,
    pub ids: &'a [i64],
    pub action: MemoryGovernanceAction,
    pub reason: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub dry_run: bool,
    pub confirm_destructive: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GovernedMemory {
    pub id: i64,
    pub title: String,
    pub previous_status: String,
    pub new_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GovernMemoryResult {
    pub dry_run: bool,
    pub action: String,
    pub reason: Option<String>,
    pub affected: Vec<GovernedMemory>,
}

pub fn govern_memories(
    conn: &Connection,
    req: &GovernMemoryRequest<'_>,
) -> Result<GovernMemoryResult> {
    let ids = unique_ids(req.ids);
    if ids.is_empty() {
        bail!("memory governance requires at least one memory id");
    }
    let reason = normalized_reason(req)?;
    let target_status = req.action.target_status();
    let tx = conn.unchecked_transaction()?;
    let mut affected = Vec::with_capacity(ids.len());
    for id in ids {
        let target = load_target(&tx, req.project, id)?;
        affected.push(GovernedMemory {
            id: target.id,
            title: target.title.clone(),
            previous_status: target.status.clone(),
            new_status: target_status.to_string(),
        });
        if req.dry_run {
            continue;
        }
        let now = chrono::Utc::now().timestamp();
        let updated = tx.execute(
            "UPDATE memories
             SET status = ?1, updated_at_epoch = ?2
             WHERE id = ?3 AND project = ?4",
            params![target_status, now, target.id, req.project],
        )?;
        if updated != 1 {
            return Err(anyhow!(
                "failed to update memory governance status: id={} project={}",
                target.id,
                req.project
            ));
        }
        insert_audit_event(&tx, req, &target, target_status, reason.as_deref(), now)?;
    }
    tx.commit()?;
    Ok(GovernMemoryResult {
        dry_run: req.dry_run,
        action: req.action.as_str().to_string(),
        reason,
        affected,
    })
}

fn unique_ids(ids: &[i64]) -> Vec<i64> {
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    ids.iter()
        .copied()
        .filter(|id| *id > 0 && seen.insert(*id))
        .collect()
}

fn normalized_reason(req: &GovernMemoryRequest<'_>) -> Result<Option<String>> {
    let reason = req.reason.map(str::trim).filter(|value| !value.is_empty());
    if req.dry_run {
        return Ok(reason.map(str::to_string));
    }
    if !req.confirm_destructive {
        bail!("memory governance mutation requires confirm_destructive=true");
    }
    let Some(reason) = reason else {
        bail!("memory governance mutation requires an explicit reason");
    };
    Ok(Some(reason.to_string()))
}

struct GovernanceTarget {
    id: i64,
    title: String,
    status: String,
}

fn load_target(conn: &Connection, project: &str, id: i64) -> Result<GovernanceTarget> {
    conn.query_row(
        "SELECT id, title, status
         FROM memories
         WHERE id = ?1 AND project = ?2",
        params![id, project],
        |row| {
            Ok(GovernanceTarget {
                id: row.get(0)?,
                title: row.get(1)?,
                status: row.get(2)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| anyhow!("memory id={} not found in project={}", id, project))
}

fn insert_audit_event(
    conn: &Connection,
    req: &GovernMemoryRequest<'_>,
    target: &GovernanceTarget,
    new_status: &str,
    reason: Option<&str>,
    now: i64,
) -> Result<()> {
    let actor = req.actor.map(str::trim).filter(|value| !value.is_empty());
    let detail = serde_json::json!({
        "action": req.action.as_str(),
        "memory_id": target.id,
        "title": target.title,
        "previous_status": target.status,
        "new_status": new_status,
        "reason": reason,
        "actor": actor,
    })
    .to_string();
    let summary = format!(
        "{} memory {}: {} -> {}{}",
        req.action.as_str(),
        target.id,
        target.status,
        new_status,
        reason
            .map(|value| format!(" ({value})"))
            .unwrap_or_default()
    );
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, detail, files, exit_code, created_at_epoch)
         VALUES (?1, ?2, 'memory_governance', ?3, ?4, NULL, NULL, ?5)",
        params![
            actor.unwrap_or("memory-governance"),
            req.project,
            summary,
            detail,
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
