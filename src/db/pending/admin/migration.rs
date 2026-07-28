use anyhow::{bail, Context, Result};
use rusqlite::{named_params, params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;

use crate::db::{self, CaptureEventInput, ExtractionTaskKind};

const AUTO_MIGRATION_RETRY_BASE_SECS: i64 = 5;
const AUTO_MIGRATION_RETRY_MAX_SECS: i64 = 900;
const AUTO_MIGRATION_RETRY_MAX_SHIFT: i64 = 8;

const AUTO_ACTIONABLE_PREDICATE: &str = "
    host IN (:claude_host, :codex_host)
    AND (
        (status = 'pending'
         AND (next_retry_epoch IS NULL OR next_retry_epoch <= :now))
        OR (status = 'processing'
            AND (lease_expires_epoch IS NULL OR lease_expires_epoch < :now))
        OR (status = 'failed'
            AND COALESCE(failure_class, 'transient') = 'transient'
            AND (next_retry_epoch IS NULL OR next_retry_epoch <= :now))
    )";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LegacyPendingMigration {
    pub pending_id: i64,
    pub event_id: String,
    pub captured_event_id: i64,
    pub extraction_task_id: i64,
    pub host: String,
    pub project: String,
    pub session_id: String,
}

pub(super) struct LegacyPendingRow {
    pub(super) id: i64,
    pub(super) host: String,
    pub(super) session_id: String,
    pub(super) project: String,
    pub(super) tool_name: String,
    pub(super) tool_input: Option<String>,
    pub(super) tool_response: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) created_at_epoch: i64,
}

pub fn count_legacy_migration_candidates(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<usize> {
    let limit = limit.max(1);
    let now = chrono::Utc::now().timestamp();
    let count: i64 = if let Some(project) = project {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE project = ?1
                   AND (status = 'pending'
                        OR (status = 'processing'
                            AND (lease_expires_epoch IS NULL OR lease_expires_epoch < ?3)))
                 ORDER BY created_at_epoch ASC, id ASC
                 LIMIT ?2
             )",
            params![project, limit, now],
            |row| row.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT id FROM pending_observations
                 WHERE status = 'pending'
                    OR (status = 'processing'
                        AND (lease_expires_epoch IS NULL OR lease_expires_epoch < ?2))
                 ORDER BY created_at_epoch ASC, id ASC
                 LIMIT ?1
             )",
            params![limit, now],
            |row| row.get(0),
        )?
    };
    Ok(count.max(0) as usize)
}

pub fn count_recoverable_archived_legacy_pending(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM pending_observations
         WHERE host IN (?1, ?2)
           AND status = 'failed'
           AND COALESCE(failure_class, 'transient') = 'transient'
           AND archived_at_epoch IS NOT NULL",
        params![
            crate::runtime_config::CLAUDE_HOST,
            crate::runtime_config::CODEX_HOST
        ],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub fn count_admin_required_archived_legacy_pending(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM pending_observations
         WHERE status = 'failed'
           AND archived_at_epoch IS NOT NULL
           AND NOT (
               host IN (?1, ?2)
               AND COALESCE(failure_class, 'transient') = 'transient'
           )",
        params![
            crate::runtime_config::CLAUDE_HOST,
            crate::runtime_config::CODEX_HOST
        ],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

pub fn migrate_legacy_pending(
    conn: &mut Connection,
    project: Option<&str>,
    fallback_host: Option<&str>,
    limit: i64,
) -> Result<Vec<LegacyPendingMigration>> {
    let fallback_host = fallback_host.map(normalize_capture_host).transpose()?;
    let tx = conn.transaction()?;
    let rows = select_legacy_pending_rows(&tx, project, limit)?;
    let mut migrated = Vec::new();

    for row in rows {
        let host = capture_host_for_row(&row.host, fallback_host)?;
        let migration = replay_legacy_row_into_capture(&tx, &row, host)?;
        let now = chrono::Utc::now().timestamp();
        let changed = tx.execute(MARK_MIGRATED_PENDING_SQL, params![row.id, now, now])?;
        if changed != 1 {
            bail!("legacy pending row {} changed while migrating", row.id);
        }
        migrated.push(migration);
    }

    tx.commit()?;
    Ok(migrated)
}

const MARK_MIGRATED_PENDING_SQL: &str = "UPDATE pending_observations
     SET status = 'migrated',
         lease_owner = NULL,
         lease_expires_epoch = NULL,
         next_retry_epoch = NULL,
         last_error = NULL,
         updated_at_epoch = ?2
     WHERE id = ?1
       AND (status = 'pending'
            OR (status = 'processing'
                AND (lease_expires_epoch IS NULL OR lease_expires_epoch < ?3)))";

pub(super) fn replay_legacy_row_into_capture(
    conn: &Connection,
    row: &LegacyPendingRow,
    host: &str,
) -> Result<LegacyPendingMigration> {
    let event_id = legacy_event_id(row.id);
    let content = legacy_capture_content(row);
    let outcome = db::record_captured_event_with_id_and_created_at(
        conn,
        &CaptureEventInput {
            host,
            session_id: &row.session_id,
            project: &row.project,
            cwd: row.cwd.as_deref(),
            event_type: "tool_result",
            role: None,
            tool_name: Some(&row.tool_name),
            content: &content,
            task_kind: Some(ExtractionTaskKind::ObservationExtract),
        },
        Some(&event_id),
        row.created_at_epoch,
    )?;
    let extraction_task_id = outcome
        .extraction_task_id
        .ok_or_else(|| anyhow::anyhow!("legacy pending migration did not enqueue extraction"))?;
    Ok(LegacyPendingMigration {
        pending_id: row.id,
        event_id,
        captured_event_id: outcome.event_row_id,
        extraction_task_id,
        host: host.to_string(),
        project: row.project.clone(),
        session_id: row.session_id.clone(),
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AutoLegacyMigrationOutcome {
    pub migrated: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AutoCandidateOutcome {
    Migrated,
    Skipped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AutoMigrationRetry {
    attempt_count: i64,
    next_retry_epoch: i64,
    backoff_secs: i64,
}

/// Worker-driven self-healing for the legacy `pending_observations` queue.
///
/// Replays actionable rows — `pending`, expired `processing`, and transient
/// `failed` — with a known capture host into the live
/// `captured_events`/`extraction_tasks` pipeline. Each row commits in its own
/// immediate transaction. Any replay failure rolls back current-pipeline
/// writes, records capped exponential backoff on the source row, and aborts the
/// batch. Automatic recovery never guesses that a shared replay error is a
/// row-local permanent failure.
pub fn auto_migrate_actionable_legacy_pending(
    conn: &mut Connection,
    limit: i64,
) -> Result<AutoLegacyMigrationOutcome> {
    let candidate_ids = select_auto_actionable_ids(conn, limit)?;
    let mut outcome = AutoLegacyMigrationOutcome::default();
    for row_id in candidate_ids {
        match auto_migrate_candidate(conn, row_id) {
            Ok(AutoCandidateOutcome::Migrated) => outcome.migrated += 1,
            Ok(AutoCandidateOutcome::Skipped) => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "legacy pending auto-migration aborted batch id={row_id} migrated_before_error={}",
                        outcome.migrated
                    )
                })
            }
        }
    }
    Ok(outcome)
}

fn select_auto_actionable_ids(conn: &Connection, limit: i64) -> Result<Vec<i64>> {
    let limit = limit.max(1);
    let now = chrono::Utc::now().timestamp();
    let sql = format!(
        "SELECT id
         FROM pending_observations
         WHERE {AUTO_ACTIONABLE_PREDICATE}
         ORDER BY created_at_epoch ASC, id ASC
         LIMIT :limit"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        named_params! {
            ":now": now,
            ":claude_host": crate::runtime_config::CLAUDE_HOST,
            ":codex_host": crate::runtime_config::CODEX_HOST,
            ":limit": limit,
        },
        |row| row.get(0),
    )?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn auto_migrate_candidate(conn: &mut Connection, row_id: i64) -> Result<AutoCandidateOutcome> {
    let mut tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("begin legacy pending auto-migration transaction")?;
    let eligibility_now = chrono::Utc::now().timestamp();
    let Some(row) = load_auto_actionable_row(&tx, row_id, eligibility_now)? else {
        tx.commit()?;
        return Ok(AutoCandidateOutcome::Skipped);
    };
    let host = normalize_capture_host(&row.host)?;
    let replay = {
        let savepoint = tx
            .savepoint_with_name("legacy_pending_auto_replay")
            .context("begin legacy pending replay savepoint")?;
        let replay = replay_legacy_row_into_capture(&savepoint, &row, host);
        match replay {
            Ok(migration) => {
                savepoint
                    .commit()
                    .context("commit legacy pending replay savepoint")?;
                Ok(migration)
            }
            Err(error) => {
                savepoint
                    .finish()
                    .context("roll back legacy pending replay savepoint")?;
                Err(error)
            }
        }
    };
    if let Err(error) = replay {
        let retry = mark_legacy_row_for_transient_retry(
            &tx,
            row_id,
            eligibility_now,
            &format!("{error:#}"),
        )
        .with_context(|| format!("record legacy pending retry state id={row_id}"))?
        .ok_or_else(|| {
            anyhow::anyhow!("legacy pending row changed inside immediate transaction id={row_id}")
        })?;
        tx.commit()
            .context("commit legacy pending retry transition")?;
        crate::log::error(
            "failure_lifecycle",
            &format!(
                "surface=pending_observation class=transient outcome=deferred id={row_id} attempt={} backoff_secs={} next_retry_epoch={}",
                retry.attempt_count, retry.backoff_secs, retry.next_retry_epoch
            ),
        );
        return Err(error).with_context(|| {
            format!(
                "legacy pending auto-migration scheduled retry id={row_id} attempt={} backoff_secs={}",
                retry.attempt_count, retry.backoff_secs
            )
        });
    }

    let completed_at = chrono::Utc::now().timestamp();
    let changed = mark_auto_migrated(&tx, row_id, eligibility_now, completed_at)?;
    match changed {
        0 => {
            tx.rollback()?;
            Ok(AutoCandidateOutcome::Skipped)
        }
        1 => {
            tx.commit()?;
            Ok(AutoCandidateOutcome::Migrated)
        }
        changed => bail!(
            "legacy pending auto-migration invariant violated: id={row_id} affected_rows={changed}"
        ),
    }
}

fn load_auto_actionable_row(
    conn: &Connection,
    row_id: i64,
    now: i64,
) -> Result<Option<LegacyPendingRow>> {
    let sql = format!(
        "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch
         FROM pending_observations
         WHERE id = :id
           AND {AUTO_ACTIONABLE_PREDICATE}"
    );
    conn.query_row(
        &sql,
        named_params! {
            ":id": row_id,
            ":now": now,
            ":claude_host": crate::runtime_config::CLAUDE_HOST,
            ":codex_host": crate::runtime_config::CODEX_HOST,
        },
        row_from_db,
    )
    .optional()
    .map_err(Into::into)
}

fn mark_auto_migrated(
    conn: &Connection,
    row_id: i64,
    eligibility_now: i64,
    completed_at: i64,
) -> Result<usize> {
    let sql = format!(
        "UPDATE pending_observations
         SET status = 'migrated',
             attempt_count = 0,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             next_retry_epoch = NULL,
             last_error = NULL,
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
             updated_at_epoch = :completed_at
         WHERE id = :id
           AND {AUTO_ACTIONABLE_PREDICATE}"
    );
    Ok(conn.execute(
        &sql,
        named_params! {
            ":id": row_id,
            ":now": eligibility_now,
            ":completed_at": completed_at,
            ":claude_host": crate::runtime_config::CLAUDE_HOST,
            ":codex_host": crate::runtime_config::CODEX_HOST,
        },
    )?)
}

fn mark_legacy_row_for_transient_retry(
    conn: &Connection,
    row_id: i64,
    now: i64,
    error: &str,
) -> Result<Option<AutoMigrationRetry>> {
    let marker = format!(
        "[auto_migration_retry] {}",
        crate::db::truncate_str(error, 1000)
    );
    let sql = format!(
        "UPDATE pending_observations
         SET status = 'failed',
             attempt_count = attempt_count + 1,
             next_retry_epoch = :now + MIN(
                 :max_retry_secs,
                 :base_retry_secs * (1 << MIN(MAX(COALESCE(attempt_count, 0), 0), :max_shift))
             ),
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             failure_class = 'transient',
             failed_at_epoch = COALESCE(failed_at_epoch, :now),
             last_error = :error,
             updated_at_epoch = :now
         WHERE id = :id
           AND {AUTO_ACTIONABLE_PREDICATE}"
    );
    let changed = conn.execute(
        &sql,
        named_params! {
            ":id": row_id,
            ":now": now,
            ":claude_host": crate::runtime_config::CLAUDE_HOST,
            ":codex_host": crate::runtime_config::CODEX_HOST,
            ":base_retry_secs": AUTO_MIGRATION_RETRY_BASE_SECS,
            ":max_retry_secs": AUTO_MIGRATION_RETRY_MAX_SECS,
            ":max_shift": AUTO_MIGRATION_RETRY_MAX_SHIFT,
            ":error": marker,
        },
    )?;
    if changed == 0 {
        return Ok(None);
    }
    let (attempt_count, next_retry_epoch): (i64, i64) = conn.query_row(
        "SELECT attempt_count, next_retry_epoch
         FROM pending_observations
         WHERE id = ?1",
        params![row_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(Some(AutoMigrationRetry {
        attempt_count,
        next_retry_epoch,
        backoff_secs: next_retry_epoch.saturating_sub(now),
    }))
}

fn select_legacy_pending_rows(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<LegacyPendingRow>> {
    let limit = limit.max(1);
    let now = chrono::Utc::now().timestamp();
    let sql = if project.is_some() {
        "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch
         FROM pending_observations
         WHERE project = ?1
           AND (status = 'pending'
                OR (status = 'processing'
                    AND (lease_expires_epoch IS NULL OR lease_expires_epoch < ?3)))
         ORDER BY created_at_epoch ASC, id ASC
         LIMIT ?2"
    } else {
        "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd, created_at_epoch
         FROM pending_observations
         WHERE status = 'pending'
            OR (status = 'processing'
                AND (lease_expires_epoch IS NULL OR lease_expires_epoch < ?2))
         ORDER BY created_at_epoch ASC, id ASC
         LIMIT ?1"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(project) = project {
        stmt.query_map(params![project, limit, now], row_from_db)?
    } else {
        stmt.query_map(params![limit, now], row_from_db)?
    };
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn row_from_db(row: &rusqlite::Row<'_>) -> rusqlite::Result<LegacyPendingRow> {
    Ok(LegacyPendingRow {
        id: row.get(0)?,
        host: row.get(1)?,
        session_id: row.get(2)?,
        project: row.get(3)?,
        tool_name: row.get(4)?,
        tool_input: row.get(5)?,
        tool_response: row.get(6)?,
        cwd: row.get(7)?,
        created_at_epoch: row.get(8)?,
    })
}

fn capture_host_for_row<'a>(row_host: &'a str, fallback_host: Option<&'a str>) -> Result<&'a str> {
    match normalize_capture_host(row_host) {
        Ok(host) => Ok(host),
        Err(_) => fallback_host
            .ok_or_else(|| anyhow::anyhow!("legacy pending row has host='{row_host}'; pass --host claude-code or --host codex-cli")),
    }
}

fn normalize_capture_host(host: &str) -> Result<&str> {
    match host {
        crate::runtime_config::CLAUDE_HOST | crate::runtime_config::CODEX_HOST => Ok(host),
        _ => bail!("invalid capture host '{host}'"),
    }
}

fn legacy_event_id(id: i64) -> String {
    format!("legacy-pending-{id}")
}

fn legacy_capture_content(row: &LegacyPendingRow) -> String {
    let git_branch = row.cwd.as_deref().and_then(db::detect_git_branch);
    serde_json::json!({
        "summary": format!("Recovered legacy {} event", row.tool_name),
        "event_type": "legacy_pending_observation",
        "detail": format!(
            "Recovered from pending_observations id={} created_at_epoch={}",
            row.id, row.created_at_epoch
        ),
        "files": serde_json::Value::Null,
        "exit_code": serde_json::Value::Null,
        "tool_name": row.tool_name,
        "tool_input": parse_jsonish(row.tool_input.as_deref()),
        "tool_response": parse_jsonish(row.tool_response.as_deref()),
        "git_branch": git_branch,
    })
    .to_string()
}

fn parse_jsonish(value: Option<&str>) -> serde_json::Value {
    match value {
        Some(value) => serde_json::from_str(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string())),
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests;
