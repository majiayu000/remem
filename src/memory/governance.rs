use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryGovernanceAction {
    Delete,
    Reject,
    MarkStale,
    AcknowledgePattern,
}

impl MemoryGovernanceAction {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_lowercase().as_str() {
            "delete" | "deleted" => Ok(Self::Delete),
            "reject" | "rejected" => Ok(Self::Reject),
            "stale" | "mark_stale" | "mark-stale" | "invalidate" => Ok(Self::MarkStale),
            "acknowledge_pattern" | "acknowledge-pattern" | "ack" => Ok(Self::AcknowledgePattern),
            other => bail!("unsupported memory governance action: {other}"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Reject => "reject",
            Self::MarkStale => "stale",
            Self::AcknowledgePattern => "acknowledge_pattern",
        }
    }

    pub fn target_status(self) -> &'static str {
        match self {
            Self::Delete => "deleted",
            Self::Reject => "rejected",
            Self::MarkStale => "stale",
            Self::AcknowledgePattern => "active",
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
    pub acknowledge_pattern: Option<&'a str>,
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

#[derive(Debug, Clone)]
pub struct GovernanceSelector<'a> {
    pub project: &'a str,
    pub query: Option<&'a str>,
    pub memory_type: Option<&'a str>,
    pub status: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebMemoryGovernanceAction {
    Archive,
    Restore,
}

impl WebMemoryGovernanceAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Restore => "restore",
        }
    }

    fn before_status(self) -> &'static str {
        match self {
            Self::Archive => "active",
            Self::Restore => "archived",
        }
    }

    fn after_status(self) -> &'static str {
        match self {
            Self::Archive => "archived",
            Self::Restore => "active",
        }
    }
}

pub struct WebMemoryGovernanceRequest<'a> {
    pub memory_id: i64,
    pub action: WebMemoryGovernanceAction,
    pub expected_version: i64,
    pub operation_id: &'a str,
    pub reason: &'a str,
    pub actor: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebMemoryGovernanceResult {
    pub memory_id: i64,
    pub project: String,
    pub before_status: String,
    pub after_status: String,
    pub version: i64,
    pub audit_id: i64,
    pub occurred_at_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebMemoryGovernanceDecision {
    Applied(WebMemoryGovernanceResult),
    NotFound,
    VersionConflict,
    NotArchivable,
    NotRecoverable,
}

pub fn govern_memory_for_web_in_transaction(
    conn: &Connection,
    req: &WebMemoryGovernanceRequest<'_>,
) -> Result<WebMemoryGovernanceDecision> {
    let target = conn
        .query_row(
            "SELECT project, title, status, version, web_archive_operation_id
             FROM memories WHERE id = ?1",
            params![req.memory_id],
            |row| {
                Ok(WebGovernanceTarget {
                    project: row.get(0)?,
                    title: row.get(1)?,
                    status: row.get(2)?,
                    version: row.get(3)?,
                    web_archive_operation_id: row.get(4)?,
                })
            },
        )
        .optional()?;
    let Some(target) = target else {
        return Ok(match req.action {
            WebMemoryGovernanceAction::Archive => WebMemoryGovernanceDecision::NotFound,
            WebMemoryGovernanceAction::Restore => WebMemoryGovernanceDecision::NotRecoverable,
        });
    };
    if target.version != req.expected_version {
        return Ok(WebMemoryGovernanceDecision::VersionConflict);
    }
    match req.action {
        WebMemoryGovernanceAction::Archive if target.status != "active" => {
            return Ok(WebMemoryGovernanceDecision::NotArchivable)
        }
        WebMemoryGovernanceAction::Restore => {
            if target.status != "archived"
                || !current_web_archive_provenance_is_valid(conn, req.memory_id, &target)?
            {
                return Ok(WebMemoryGovernanceDecision::NotRecoverable);
            }
        }
        WebMemoryGovernanceAction::Archive => {}
    }

    let occurred_at_epoch = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE memories
         SET status = ?1, updated_at_epoch = ?2
         WHERE id = ?3 AND version = ?4 AND status = ?5",
        params![
            req.action.after_status(),
            occurred_at_epoch,
            req.memory_id,
            req.expected_version,
            req.action.before_status()
        ],
    )?;
    if updated != 1 {
        bail!("web memory governance guarded update did not affect exactly one row");
    }
    if req.action == WebMemoryGovernanceAction::Archive {
        let marker_updated = conn.execute(
            "UPDATE memories SET web_archive_operation_id = ?1 WHERE id = ?2 AND status = 'archived'",
            params![req.operation_id, req.memory_id],
        )?;
        if marker_updated != 1 {
            bail!("web archive marker update did not affect exactly one row");
        }
    }
    let (after_status, version, marker): (String, i64, Option<String>) = conn.query_row(
        "SELECT status, version, web_archive_operation_id FROM memories WHERE id = ?1",
        params![req.memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if after_status != req.action.after_status()
        || (req.action == WebMemoryGovernanceAction::Archive
            && marker.as_deref() != Some(req.operation_id))
        || (req.action == WebMemoryGovernanceAction::Restore && marker.is_some())
    {
        bail!("web memory governance postcondition failed");
    }
    let audit_id = insert_web_audit_event(conn, req, &target, &after_status, occurred_at_epoch)?;
    Ok(WebMemoryGovernanceDecision::Applied(
        WebMemoryGovernanceResult {
            memory_id: req.memory_id,
            project: target.project,
            before_status: target.status,
            after_status,
            version,
            audit_id,
            occurred_at_epoch,
        },
    ))
}

struct WebGovernanceTarget {
    project: String,
    title: String,
    status: String,
    version: i64,
    web_archive_operation_id: Option<String>,
}

fn current_web_archive_provenance_is_valid(
    conn: &Connection,
    memory_id: i64,
    target: &WebGovernanceTarget,
) -> Result<bool> {
    let Some(marker) = target.web_archive_operation_id.as_deref() else {
        return Ok(false);
    };
    let evidence = conn
        .query_row(
            "SELECT r.audit_id, r.response_json, e.event_type, e.project, e.detail
             FROM api_mutation_requests r
             JOIN events e ON e.id = r.audit_id
             WHERE r.operation_id = ?1
               AND r.resource_kind = 'memory'
               AND r.resource_id = ?2
               AND r.action = 'archive'
               AND r.response_schema_version = 1",
            params![marker, memory_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((audit_id, response_json, event_type, project, detail)) = evidence else {
        return Ok(false);
    };
    if event_type != "memory_governance" || project != target.project {
        return Ok(false);
    }
    let response: serde_json::Value = match serde_json::from_str(&response_json) {
        Ok(response) => response,
        Err(_) => return Ok(false),
    };
    let detail: serde_json::Value = match detail.as_deref().map(serde_json::from_str).transpose() {
        Ok(Some(detail)) => detail,
        _ => return Ok(false),
    };
    Ok(response["operation_id"] == marker
        && response["response_schema_version"] == 1
        && response["audit_id"] == audit_id
        && response["memory_id"] == memory_id
        && response["action"] == "archive"
        && response["before_status"] == "active"
        && response["after_status"] == "archived"
        && detail["operation_id"] == marker
        && detail["memory_id"] == memory_id
        && detail["action"] == "archive"
        && detail["previous_status"] == "active"
        && detail["new_status"] == "archived")
}

fn insert_web_audit_event(
    conn: &Connection,
    req: &WebMemoryGovernanceRequest<'_>,
    target: &WebGovernanceTarget,
    new_status: &str,
    now: i64,
) -> Result<i64> {
    let detail = serde_json::json!({
        "action": req.action.as_str(),
        "memory_id": req.memory_id,
        "title": target.title,
        "previous_status": target.status,
        "new_status": new_status,
        "reason": req.reason,
        "actor": req.actor,
        "operation_id": req.operation_id,
    })
    .to_string();
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, detail, files, exit_code, created_at_epoch)
         VALUES (?1, ?2, 'memory_governance', ?3, ?4, NULL, NULL, ?5)",
        params![
            format!("api:{}", req.operation_id),
            target.project,
            format!("Web {} memory {}", req.action.as_str(), req.memory_id),
            detail,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn select_memory_ids(conn: &Connection, selector: &GovernanceSelector<'_>) -> Result<Vec<i64>> {
    let mut conditions = vec!["project = ?1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(selector.project.to_string())];
    let mut idx = 2;

    if let Some(status) = normalized_status_filter(selector.status)? {
        conditions.push(format!("status = ?{idx}"));
        params.push(Box::new(status));
        idx += 1;
    }

    if let Some(memory_type) = trimmed(selector.memory_type) {
        conditions.push(format!("memory_type = ?{idx}"));
        params.push(Box::new(memory_type.to_string()));
        idx += 1;
    }

    if let Some(query) = trimmed(selector.query) {
        let pattern = like_pattern(query);
        conditions.push(format!(
            "(title LIKE ?{idx} ESCAPE '\\' \
             OR content LIKE ?{next_idx} ESCAPE '\\' \
             OR COALESCE(search_context, '') LIKE ?{third_idx} ESCAPE '\\')",
            idx = idx,
            next_idx = idx + 1,
            third_idx = idx + 2
        ));
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern));
        idx += 3;
    }

    params.push(Box::new(selector.limit.max(1)));
    params.push(Box::new(selector.offset.max(0)));
    let sql = format!(
        "SELECT id FROM memories \
         WHERE {} \
         ORDER BY updated_at_epoch DESC, id DESC \
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "),
        idx,
        idx + 1
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    crate::db::query::collect_rows(rows)
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
    let acknowledged_pattern = normalized_acknowledge_pattern(req)?;
    let target_status = req.action.target_status();
    let tx = conn.unchecked_transaction()?;
    let mut affected = Vec::with_capacity(ids.len());
    let mut rule_source_ids = Vec::new();
    for id in ids {
        let target = load_target(&tx, req.project, id)?;
        if req.action == MemoryGovernanceAction::AcknowledgePattern {
            validate_acknowledgement(&target, acknowledged_pattern)?;
        }
        let new_status = if req.action == MemoryGovernanceAction::AcknowledgePattern {
            target.status.as_str()
        } else {
            target_status
        };
        affected.push(GovernedMemory {
            id: target.id,
            title: target.title.clone(),
            previous_status: target.status.clone(),
            new_status: new_status.to_string(),
        });
        if req.dry_run {
            continue;
        }
        if req.action != MemoryGovernanceAction::AcknowledgePattern {
            rule_source_ids.push(target.id);
        }
        let now = chrono::Utc::now().timestamp();
        let updated = if req.action == MemoryGovernanceAction::AcknowledgePattern {
            tx.execute(
                "UPDATE memories
                 SET acknowledged_pattern_id = ?1,
                     acknowledged_pattern_version = ?2,
                     acknowledged_at_epoch = ?3,
                     updated_at_epoch = ?3
                 WHERE id = ?4 AND project = ?5",
                params![
                    acknowledged_pattern,
                    crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
                    now,
                    target.id,
                    req.project
                ],
            )?
        } else {
            tx.execute(
                "UPDATE memories
                 SET status = ?1, updated_at_epoch = ?2
                 WHERE id = ?3 AND project = ?4",
                params![target_status, now, target.id, req.project],
            )?
        };
        if updated != 1 {
            return Err(anyhow!(
                "failed to update memory governance target: id={} project={}",
                target.id,
                req.project
            ));
        }
        insert_audit_event(&tx, req, &target, new_status, reason.as_deref(), now)?;
    }
    crate::memory::preference::compilation::enqueue_for_memory_ids(&tx, &rule_source_ids)?;
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

fn trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalized_status_filter(status: Option<&str>) -> Result<Option<String>> {
    let Some(status) = trimmed(status) else {
        return Ok(Some("active".to_string()));
    };
    let normalized = status.to_lowercase();
    if matches!(normalized.as_str(), "all" | "*") {
        return Ok(None);
    }
    if matches!(
        normalized.as_str(),
        "active" | "stale" | "rejected" | "deleted" | "archived" | "superseded"
    ) {
        return Ok(Some(normalized));
    }
    bail!("unsupported memory status filter: {status}");
}

fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
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

fn normalized_acknowledge_pattern<'a>(req: &'a GovernMemoryRequest<'a>) -> Result<Option<&'a str>> {
    let pattern = req
        .acknowledge_pattern
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if req.action == MemoryGovernanceAction::AcknowledgePattern && pattern.is_none() {
        bail!("acknowledge_pattern action requires acknowledge_pattern");
    }
    if req.action != MemoryGovernanceAction::AcknowledgePattern && pattern.is_some() {
        bail!("acknowledge_pattern is only valid with acknowledge_pattern action");
    }
    Ok(pattern)
}

fn validate_acknowledgement(
    target: &GovernanceTarget,
    acknowledged_pattern: Option<&str>,
) -> Result<()> {
    let acknowledged_pattern = acknowledged_pattern.expect("validated acknowledge_pattern");
    let Some(matched) = crate::memory::poisoning::scan_instruction_pattern(&format!(
        "{}\n{}",
        target.title, target.content
    )) else {
        bail!(
            "memory id={} does not match an instruction-pattern; cannot acknowledge {}",
            target.id,
            acknowledged_pattern
        );
    };
    if matched.pattern_id != acknowledged_pattern {
        bail!(
            "memory id={} acknowledged pattern {} does not match instruction-pattern {}@v{}",
            target.id,
            acknowledged_pattern,
            matched.pattern_id,
            matched.pattern_set_version
        );
    }
    Ok(())
}

struct GovernanceTarget {
    id: i64,
    title: String,
    content: String,
    status: String,
}

fn load_target(conn: &Connection, project: &str, id: i64) -> Result<GovernanceTarget> {
    conn.query_row(
        "SELECT id, title, content, status
         FROM memories
         WHERE id = ?1 AND project = ?2",
        params![id, project],
        |row| {
            Ok(GovernanceTarget {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                status: row.get(3)?,
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
        "acknowledged_pattern": req.acknowledge_pattern,
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
#[cfg(test)]
mod web_tests;
