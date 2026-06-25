use std::collections::HashSet;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;

const ACTIVE_STATUS: &str = "active";
const DEFAULT_ACTOR: &str = "cli";
const DEFAULT_REASON: &str = "manual suppression";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuppressionTarget {
    pub kind: String,
    pub id: Option<i64>,
    pub value: Option<String>,
}

impl SuppressionTarget {
    pub fn label(&self) -> String {
        match (self.id, self.value.as_deref()) {
            (Some(id), _) => format!("{}:{id}", self.kind),
            (None, Some(value)) => format!("{}:{value}", self.kind),
            (None, None) => self.kind.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SuppressionRecord {
    pub id: i64,
    pub owner_scope: Option<String>,
    pub owner_key: Option<String>,
    pub target_kind: String,
    pub target_id: Option<i64>,
    pub target_value: Option<String>,
    pub reason: String,
    pub actor: String,
    pub status: String,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackRecord {
    pub id: i64,
    pub target_kind: String,
    pub target_id: Option<i64>,
    pub target_value: Option<String>,
    pub feedback: String,
    pub source: String,
    pub context_injection_item_id: Option<i64>,
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub reason: Option<String>,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone)]
pub struct SuppressRequest<'a> {
    pub target: SuppressionTarget,
    pub reason: Option<&'a str>,
    pub actor: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct FeedbackRequest<'a> {
    pub target: SuppressionTarget,
    pub feedback: &'a str,
    pub source: Option<&'a str>,
    pub context_injection_item_id: Option<i64>,
    pub session_id: Option<&'a str>,
    pub project: Option<&'a str>,
    pub reason: Option<&'a str>,
}

pub fn parse_target(raw: &str) -> Result<SuppressionTarget> {
    let input = raw.trim();
    if input.is_empty() {
        bail!("suppression target cannot be empty");
    }
    if let Ok(id) = input.parse::<i64>() {
        if id <= 0 {
            bail!("memory target id must be positive");
        }
        return Ok(SuppressionTarget {
            kind: "memory".to_string(),
            id: Some(id),
            value: None,
        });
    }

    let Some((kind_raw, value_raw)) = input.split_once(':') else {
        return Ok(SuppressionTarget {
            kind: "topic_key".to_string(),
            id: None,
            value: Some(input.to_string()),
        });
    };
    let kind = normalize_kind(kind_raw)?;
    let value = value_raw.trim();
    if value.is_empty() {
        bail!("suppression target value cannot be empty");
    }

    if id_target_kind(&kind) {
        let id = value
            .parse::<i64>()
            .with_context(|| format!("{kind} target requires an integer id"))?;
        if id <= 0 {
            bail!("{kind} target id must be positive");
        }
        return Ok(SuppressionTarget {
            kind,
            id: Some(id),
            value: None,
        });
    }

    Ok(SuppressionTarget {
        kind,
        id: None,
        value: Some(value.to_string()),
    })
}

pub fn memory_policy_filter_sql(alias: &str) -> String {
    format!(
        "NOT EXISTS (
             SELECT 1
             FROM memory_suppressions ms
             WHERE ms.status = 'active'
               AND (
                    (ms.target_kind = 'memory' AND ms.target_id = {alias}.id)
                 OR (ms.target_kind = 'topic_key'
                     AND ms.target_value IS NOT NULL
                     AND {alias}.topic_key = ms.target_value)
                 OR (ms.target_kind = 'entity'
                     AND ms.target_value IS NOT NULL
                     AND EXISTS (
                         SELECT 1
                         FROM memory_entities ms_me
                         JOIN entities ms_e ON ms_e.id = ms_me.entity_id
                         WHERE ms_me.memory_id = {alias}.id
                           AND lower(ms_e.canonical_name) = lower(ms.target_value)
                     ))
                 OR (ms.target_kind = 'pattern'
                     AND ms.target_value IS NOT NULL
                     AND (
                         instr(lower({alias}.title), lower(ms.target_value)) > 0
                      OR instr(lower({alias}.content), lower(ms.target_value)) > 0
                     ))
               )
         )"
    )
}

pub fn user_claim_policy_filter_sql(alias: &str) -> String {
    format!(
        "NOT EXISTS (
             SELECT 1
             FROM memory_suppressions ms
             WHERE ms.status = 'active'
               AND (
                    (ms.target_kind = 'user_claim' AND ms.target_id = {alias}.id)
                 OR (ms.target_kind = 'pattern'
                     AND ms.target_value IS NOT NULL
                     AND (
                         instr(lower({alias}.claim_text), lower(ms.target_value)) > 0
                      OR instr(lower({alias}.claim_key), lower(ms.target_value)) > 0
                     ))
               )
         )"
    )
}

pub fn user_claim_is_policy_suppressed(conn: &Connection, claim_id: i64) -> Result<bool> {
    let sql = format!(
        "SELECT NOT ({}) FROM user_context_claims WHERE id = ?1",
        user_claim_policy_filter_sql("user_context_claims")
    );
    conn.query_row(&sql, [claim_id], |row| row.get::<_, bool>(0))
        .optional()?
        .context("user-context claim disappeared during suppression check")
}

pub fn active_suppressed_memory_ids(conn: &Connection, ids: &[i64]) -> Result<HashSet<i64>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let placeholders = (1..=ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT m.id
         FROM memories m
         WHERE m.id IN ({placeholders})
           AND NOT ({})",
        memory_policy_filter_sql("m")
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |row| row.get::<_, i64>(0))?;
    let suppressed = crate::db::query::collect_rows(rows)?;
    Ok(suppressed.into_iter().collect())
}

pub fn has_active_suppressions(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_suppressions WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn create_suppression(
    conn: &Connection,
    req: &SuppressRequest<'_>,
) -> Result<SuppressionRecord> {
    validate_target(&req.target)?;
    let reason = normalize_text(req.reason, DEFAULT_REASON)?;
    let actor = normalize_text(req.actor, DEFAULT_ACTOR)?;
    if let Some(existing) = load_active_suppression_for_target(conn, &req.target)? {
        return Ok(existing);
    }
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_suppressions
         (owner_scope, owner_key, target_kind, target_id, target_value, reason, actor,
          status, created_at_epoch, updated_at_epoch)
         VALUES (NULL, NULL, ?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)",
        params![
            req.target.kind,
            req.target.id,
            req.target.value,
            reason,
            actor,
            now
        ],
    )
    .context("insert memory suppression")?;
    load_suppression(conn, conn.last_insert_rowid())
}

pub fn revoke_suppression_arg(
    conn: &Connection,
    arg: &str,
    reason: Option<&str>,
    actor: Option<&str>,
) -> Result<Vec<SuppressionRecord>> {
    let actor = normalize_text(actor, DEFAULT_ACTOR)?;
    let reason = normalize_text(reason, "manual unsuppression")?;
    if let Ok(id) = arg.trim().parse::<i64>() {
        if let Some(record) = load_suppression_optional(conn, id)? {
            if record.status != ACTIVE_STATUS {
                bail!("suppression {id} is already {}", record.status);
            }
            return revoke_suppression_ids(conn, &[id], &reason, &actor);
        }
    }
    let target = parse_target(arg)?;
    let active = active_suppressions_for_target(conn, &target)?;
    if active.is_empty() {
        bail!("no active suppression found for {}", target.label());
    }
    let ids = active.iter().map(|record| record.id).collect::<Vec<_>>();
    revoke_suppression_ids(conn, &ids, &reason, &actor)
}

pub fn record_feedback(conn: &Connection, req: &FeedbackRequest<'_>) -> Result<FeedbackRecord> {
    validate_target(&req.target)?;
    let feedback = normalize_feedback(req.feedback)?;
    let source = normalize_text(req.source, DEFAULT_ACTOR)?;
    let reason = optional_trimmed(req.reason);
    let session_id = optional_trimmed(req.session_id);
    let project = optional_trimmed(req.project);
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_feedback
         (target_kind, target_id, target_value, feedback, source,
          context_injection_item_id, session_id, project, reason, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            req.target.kind,
            req.target.id,
            req.target.value,
            feedback,
            source,
            req.context_injection_item_id,
            session_id,
            project,
            reason,
            now,
        ],
    )
    .context("insert memory feedback")?;
    load_feedback(conn, conn.last_insert_rowid())
}

pub fn list_suppressions(
    conn: &Connection,
    include_inactive: bool,
) -> Result<Vec<SuppressionRecord>> {
    let sql = if include_inactive {
        "SELECT id, owner_scope, owner_key, target_kind, target_id, target_value,
                reason, actor, status, created_at_epoch, updated_at_epoch
         FROM memory_suppressions
         ORDER BY updated_at_epoch DESC, id DESC"
    } else {
        "SELECT id, owner_scope, owner_key, target_kind, target_id, target_value,
                reason, actor, status, created_at_epoch, updated_at_epoch
         FROM memory_suppressions
         WHERE status = 'active'
         ORDER BY updated_at_epoch DESC, id DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], suppression_from_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn active_suppressions_for_memory(
    conn: &Connection,
    memory_id: i64,
) -> Result<Vec<SuppressionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT ms.id, ms.owner_scope, ms.owner_key, ms.target_kind, ms.target_id,
                ms.target_value, ms.reason, ms.actor, ms.status,
                ms.created_at_epoch, ms.updated_at_epoch
         FROM memory_suppressions ms
         JOIN memories m ON m.id = ?1
         WHERE ms.status = 'active'
           AND (
                (ms.target_kind = 'memory' AND ms.target_id = m.id)
             OR (ms.target_kind = 'topic_key'
                 AND ms.target_value IS NOT NULL
                 AND m.topic_key = ms.target_value)
             OR (ms.target_kind = 'entity'
                 AND ms.target_value IS NOT NULL
                 AND EXISTS (
                     SELECT 1
                     FROM memory_entities ms_me
                     JOIN entities ms_e ON ms_e.id = ms_me.entity_id
                     WHERE ms_me.memory_id = m.id
                       AND lower(ms_e.canonical_name) = lower(ms.target_value)
                 ))
             OR (ms.target_kind = 'pattern'
                 AND ms.target_value IS NOT NULL
                 AND (
                     instr(lower(m.title), lower(ms.target_value)) > 0
                  OR instr(lower(m.content), lower(ms.target_value)) > 0
                 ))
           )
         ORDER BY ms.updated_at_epoch DESC, ms.id DESC",
    )?;
    let rows = stmt.query_map([memory_id], suppression_from_row)?;
    crate::db::query::collect_rows(rows)
}

fn revoke_suppression_ids(
    conn: &Connection,
    ids: &[i64],
    reason: &str,
    actor: &str,
) -> Result<Vec<SuppressionRecord>> {
    let now = chrono::Utc::now().timestamp();
    let tx = conn.unchecked_transaction()?;
    for id in ids {
        tx.execute(
            "UPDATE memory_suppressions
             SET status = 'revoked',
                 reason = ?1,
                 actor = ?2,
                 updated_at_epoch = ?3
             WHERE id = ?4 AND status = 'active'",
            params![reason, actor, now, id],
        )?;
    }
    let mut revoked = Vec::new();
    for id in ids {
        revoked.push(load_suppression(&tx, *id)?);
    }
    tx.commit()?;
    Ok(revoked)
}

fn load_active_suppression_for_target(
    conn: &Connection,
    target: &SuppressionTarget,
) -> Result<Option<SuppressionRecord>> {
    let mut active = active_suppressions_for_target(conn, target)?;
    Ok(active.pop())
}

fn active_suppressions_for_target(
    conn: &Connection,
    target: &SuppressionTarget,
) -> Result<Vec<SuppressionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, owner_scope, owner_key, target_kind, target_id, target_value,
                reason, actor, status, created_at_epoch, updated_at_epoch
         FROM memory_suppressions
         WHERE status = 'active'
           AND target_kind = ?1
           AND (
                (target_id IS NOT NULL AND target_id = ?2)
             OR (target_value IS NOT NULL AND target_value = ?3)
           )
         ORDER BY updated_at_epoch DESC, id DESC",
    )?;
    let rows = stmt.query_map(
        params![target.kind, target.id, target.value],
        suppression_from_row,
    )?;
    crate::db::query::collect_rows(rows)
}

fn load_suppression(conn: &Connection, id: i64) -> Result<SuppressionRecord> {
    load_suppression_optional(conn, id)?.ok_or_else(|| anyhow!("suppression {id} not found"))
}

fn load_suppression_optional(conn: &Connection, id: i64) -> Result<Option<SuppressionRecord>> {
    conn.query_row(
        "SELECT id, owner_scope, owner_key, target_kind, target_id, target_value,
                reason, actor, status, created_at_epoch, updated_at_epoch
         FROM memory_suppressions
         WHERE id = ?1",
        [id],
        suppression_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn load_feedback(conn: &Connection, id: i64) -> Result<FeedbackRecord> {
    conn.query_row(
        "SELECT id, target_kind, target_id, target_value, feedback, source,
                context_injection_item_id, session_id, project, reason, created_at_epoch
         FROM memory_feedback
         WHERE id = ?1",
        [id],
        feedback_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow!("feedback {id} not found"))
}

fn suppression_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SuppressionRecord> {
    Ok(SuppressionRecord {
        id: row.get(0)?,
        owner_scope: row.get(1)?,
        owner_key: row.get(2)?,
        target_kind: row.get(3)?,
        target_id: row.get(4)?,
        target_value: row.get(5)?,
        reason: row.get(6)?,
        actor: row.get(7)?,
        status: row.get(8)?,
        created_at_epoch: row.get(9)?,
        updated_at_epoch: row.get(10)?,
    })
}

fn feedback_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackRecord> {
    Ok(FeedbackRecord {
        id: row.get(0)?,
        target_kind: row.get(1)?,
        target_id: row.get(2)?,
        target_value: row.get(3)?,
        feedback: row.get(4)?,
        source: row.get(5)?,
        context_injection_item_id: row.get(6)?,
        session_id: row.get(7)?,
        project: row.get(8)?,
        reason: row.get(9)?,
        created_at_epoch: row.get(10)?,
    })
}

fn validate_target(target: &SuppressionTarget) -> Result<()> {
    normalize_kind(&target.kind)?;
    match target.kind.as_str() {
        "memory" | "user_claim" | "user_candidate" => {
            if target.id.is_none() {
                bail!("{} suppression target requires an id", target.kind);
            }
        }
        "topic_key" | "entity" | "pattern" => {
            if target
                .value
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                bail!("{} suppression target requires a value", target.kind);
            }
        }
        "summary" => {
            if target.id.is_none() && target.value.as_deref().is_none_or(str::is_empty) {
                bail!("summary suppression target requires an id or value");
            }
        }
        _ => unreachable!("normalize_kind accepted an unknown target kind"),
    }
    Ok(())
}

fn normalize_kind(raw: &str) -> Result<String> {
    let normalized = raw.trim().replace('-', "_");
    let kind = match normalized.as_str() {
        "memory" | "mem" => "memory",
        "claim" | "user_claim" | "user_context_claim" => "user_claim",
        "candidate" | "user_candidate" => "user_candidate",
        "topic" | "topic_key" => "topic_key",
        "entity" => "entity",
        "pattern" => "pattern",
        "summary" | "summary_line" => "summary",
        _ => bail!("unsupported suppression target kind: {raw}"),
    };
    Ok(kind.to_string())
}

fn id_target_kind(kind: &str) -> bool {
    matches!(kind, "memory" | "user_claim" | "user_candidate")
}

fn normalize_feedback(raw: &str) -> Result<&'static str> {
    match raw.trim().replace('-', "_").as_str() {
        "relevant" => Ok("relevant"),
        "not_relevant" => Ok("not_relevant"),
        "harmful" => Ok("harmful"),
        "stale" => Ok("stale"),
        "too_noisy" => Ok("too_noisy"),
        _ => bail!("unsupported feedback value: {raw}"),
    }
}

fn normalize_text<'a>(value: Option<&'a str>, default: &'a str) -> Result<String> {
    let normalized = value.unwrap_or(default).trim();
    if normalized.is_empty() {
        bail!("value cannot be empty");
    }
    Ok(normalized.to_string())
}

fn optional_trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::{params, Connection};

    use super::*;

    #[test]
    fn parse_target_accepts_memory_claim_and_text_keys() -> Result<()> {
        assert_eq!(
            parse_target("42")?,
            SuppressionTarget {
                kind: "memory".to_string(),
                id: Some(42),
                value: None,
            }
        );
        assert_eq!(
            parse_target("claim:7")?,
            SuppressionTarget {
                kind: "user_claim".to_string(),
                id: Some(7),
                value: None,
            }
        );
        assert_eq!(
            parse_target("topic:rust")?,
            SuppressionTarget {
                kind: "topic_key".to_string(),
                id: None,
                value: Some("rust".to_string()),
            }
        );
        assert_eq!(
            parse_target("rust")?,
            SuppressionTarget {
                kind: "topic_key".to_string(),
                id: None,
                value: Some("rust".to_string()),
            }
        );
        Ok(())
    }

    #[test]
    fn suppression_records_and_revokes_policy_without_deleting_memory() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, topic_key, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (1, '/repo', 'topic-a', 'Suppressed', 'body', 'decision', 10, 10, 'active')",
            [],
        )?;
        let target = parse_target("memory:1")?;
        let record = create_suppression(
            &conn,
            &SuppressRequest {
                target: target.clone(),
                reason: Some("stale"),
                actor: Some("test"),
            },
        )?;
        assert_eq!(record.status, "active");
        assert_eq!(active_suppressed_memory_ids(&conn, &[1])?.len(), 1);
        let still_exists: i64 =
            conn.query_row("SELECT COUNT(*) FROM memories WHERE id = 1", [], |row| {
                row.get(0)
            })?;
        assert_eq!(still_exists, 1);

        let revoked = revoke_suppression_arg(&conn, &record.id.to_string(), None, None)?;
        assert_eq!(revoked[0].status, "revoked");
        assert!(active_suppressed_memory_ids(&conn, &[1])?.is_empty());
        Ok(())
    }

    #[test]
    fn entity_and_pattern_suppressions_match_memory_policy_filter() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (1, '/repo', 'Graphiti note', 'entity body', 'decision', 10, 10, 'active'),
                    (2, '/repo', 'Other', 'contains private phrase', 'decision', 11, 11, 'active')",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO entities(id, canonical_name, entity_type, created_at_epoch)
             VALUES (1, 'Graphiti', 'tool', 10)",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO memory_entities(memory_id, entity_id)
             VALUES (1, 1)",
            [],
        )?;
        create_suppression(
            &conn,
            &SuppressRequest {
                target: parse_target("entity:graphiti")?,
                reason: None,
                actor: None,
            },
        )?;
        create_suppression(
            &conn,
            &SuppressRequest {
                target: parse_target("pattern:private phrase")?,
                reason: None,
                actor: None,
            },
        )?;
        let rows: Vec<i64> = {
            let sql = format!(
                "SELECT m.id FROM memories m WHERE {} ORDER BY m.id",
                memory_policy_filter_sql("m")
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            crate::db::query::collect_rows(rows)?
        };
        assert!(rows.is_empty());
        Ok(())
    }

    #[test]
    fn feedback_records_event_without_mutating_target() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (1, '/repo', 'Feedback target', 'body', 'decision', 10, 10, 'active')",
            [],
        )?;
        let feedback = record_feedback(
            &conn,
            &FeedbackRequest {
                target: parse_target("memory:1")?,
                feedback: "not-relevant",
                source: Some("test"),
                context_injection_item_id: None,
                session_id: Some("s1"),
                project: Some("/repo"),
                reason: Some("wrong task"),
            },
        )?;
        assert_eq!(feedback.feedback, "not_relevant");
        let status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![1],
            |row| row.get(0),
        )?;
        assert_eq!(status, "active");
        Ok(())
    }
}
