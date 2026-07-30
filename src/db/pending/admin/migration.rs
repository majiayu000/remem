use std::collections::HashSet;

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

const MANUAL_ELIGIBLE_PREDICATE: &str = "
    (status = 'pending'
     OR (status = 'processing'
         AND (lease_expires_epoch IS NULL OR lease_expires_epoch < :now)))";

const LEGACY_PENDING_SNAPSHOT_COLUMNS: &str = "
    id, host, session_id, project, tool_name, tool_input, tool_response, cwd,
    created_at_epoch, updated_at_epoch, status, attempt_count, next_retry_epoch,
    last_error, lease_owner, lease_expires_epoch, failure_class, failed_at_epoch,
    archived_at_epoch";

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

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub(super) struct PreparedLegacyReplay {
    snapshot: LegacyPendingRow,
    content: String,
    git_branch: Option<String>,
}

#[derive(PartialEq, Eq)]
struct LegacyPendingSnapshot {
    legacy: LegacyPendingRow,
    updated_at_epoch: i64,
    status: String,
    attempt_count: i64,
    next_retry_epoch: Option<i64>,
    last_error: Option<String>,
    lease_owner: Option<String>,
    lease_expires_epoch: Option<i64>,
    failure_class: Option<String>,
    failed_at_epoch: Option<i64>,
    archived_at_epoch: Option<i64>,
}

struct PreparedManualLegacyReplay {
    source: LegacyPendingSnapshot,
    replay: PreparedLegacyReplay,
    host: String,
}

impl PreparedLegacyReplay {
    pub(super) fn matches(&self, row: &LegacyPendingRow) -> bool {
        &self.snapshot == row
    }
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
    let now = chrono::Utc::now().timestamp();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM pending_observations
         WHERE host IN (?1, ?2)
           AND status = 'failed'
           AND COALESCE(failure_class, 'transient') = 'transient'
           AND archived_at_epoch IS NOT NULL
           AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?3)",
        params![
            crate::runtime_config::CLAUDE_HOST,
            crate::runtime_config::CODEX_HOST,
            now,
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
    let mut detector = db::detect_git_branch;
    migrate_legacy_pending_with_detector(conn, project, fallback_host, limit, &mut detector)
}

fn migrate_legacy_pending_with_detector(
    conn: &mut Connection,
    project: Option<&str>,
    fallback_host: Option<&str>,
    limit: i64,
    detector: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<Vec<LegacyPendingMigration>> {
    let fallback_host = fallback_host.map(normalize_capture_host).transpose()?;
    let rows = select_legacy_pending_rows(conn, project, limit)?;
    let prepared = rows
        .into_iter()
        .map(|source| {
            let host = capture_host_for_row(&source.legacy.host, fallback_host)?.to_string();
            let replay = prepare_legacy_replay_with_detector(&source.legacy, detector);
            Ok(PreparedManualLegacyReplay {
                source,
                replay,
                host,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if prepared.is_empty() {
        return Ok(Vec::new());
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("begin manual legacy pending migration transaction")?;
    let mut migrated = Vec::new();

    for prepared_row in prepared {
        let pending_id = prepared_row.source.legacy.id;
        let eligibility_now = chrono::Utc::now().timestamp();
        let row = load_legacy_pending_row(&tx, pending_id, eligibility_now)?.ok_or_else(|| {
            anyhow::anyhow!(
                "legacy pending row {pending_id} changed or became ineligible while preparing migration; batch replay was rolled back"
            )
        })?;
        if prepared_row.source != row || !prepared_row.replay.matches(&row.legacy) {
            bail!(
                "legacy pending row {pending_id} changed while preparing migration; batch replay was rolled back"
            );
        }
        let migration =
            replay_prepared_legacy_row_into_capture(&tx, &prepared_row.replay, &prepared_row.host)
                .with_context(|| format!("replay legacy pending row {pending_id}"))?;
        let completed_at = chrono::Utc::now().timestamp();
        let changed = tx.execute(
            MARK_MIGRATED_PENDING_SQL,
            params![pending_id, completed_at, eligibility_now],
        )?;
        if changed != 1 {
            bail!(
                "legacy pending row {pending_id} changed while migrating; batch replay was rolled back"
            );
        }
        migrated.push(migration);
    }

    tx.commit()
        .context("commit manual legacy pending migration")?;
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

pub(super) fn prepare_legacy_replay_with_detector(
    row: &LegacyPendingRow,
    detector: &mut dyn FnMut(&str) -> Option<String>,
) -> PreparedLegacyReplay {
    let git_branch = row.cwd.as_deref().and_then(detector);
    let content = legacy_capture_content(row, git_branch.as_deref());
    PreparedLegacyReplay {
        snapshot: row.clone(),
        content,
        git_branch,
    }
}

pub(super) fn replay_prepared_legacy_row_into_capture(
    conn: &Connection,
    prepared: &PreparedLegacyReplay,
    host: &str,
) -> Result<LegacyPendingMigration> {
    let row = &prepared.snapshot;
    let event_id = legacy_event_id(row.id);
    let outcome =
        db::capture::record_captured_event_with_id_and_created_at_and_precomputed_git_branch(
            conn,
            &CaptureEventInput {
                host,
                session_id: &row.session_id,
                project: &row.project,
                cwd: row.cwd.as_deref(),
                event_type: "tool_result",
                role: None,
                tool_name: Some(&row.tool_name),
                content: &prepared.content,
                task_kind: Some(ExtractionTaskKind::ObservationExtract),
            },
            Some(&event_id),
            row.created_at_epoch,
            prepared.git_branch.as_deref(),
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
    Migrated {
        extraction_task_id: i64,
        captured_event_id: i64,
    },
    Skipped,
    YieldedToCurrentWork,
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
    let mut detector = db::detect_git_branch;
    auto_migrate_actionable_legacy_pending_with_detector(conn, limit, &mut detector)
}

pub(super) fn auto_migrate_actionable_legacy_pending_with_detector(
    conn: &mut Connection,
    limit: i64,
    detector: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<AutoLegacyMigrationOutcome> {
    let candidate_ids = select_auto_actionable_ids(conn, limit)?;
    let mut outcome = AutoLegacyMigrationOutcome::default();
    let mut migrated_tasks = HashSet::new();
    for row_id in candidate_ids {
        if current_extraction_work_is_ready(conn, &migrated_tasks)? {
            break;
        }
        match auto_migrate_candidate_with_detector(
            conn,
            row_id,
            detector,
            &migrated_tasks,
        ) {
            Ok(AutoCandidateOutcome::Migrated {
                extraction_task_id,
                captured_event_id,
            }) => {
                outcome.migrated += 1;
                migrated_tasks.insert((extraction_task_id, captured_event_id));
            }
            Ok(AutoCandidateOutcome::Skipped) => {}
            Ok(AutoCandidateOutcome::YieldedToCurrentWork) => break,
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

fn current_extraction_work_is_ready(
    conn: &Connection,
    migrated_tasks: &HashSet<(i64, i64)>,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare(
        "SELECT id, status, high_watermark_event_id, next_retry_epoch
         FROM extraction_tasks
         WHERE (status = 'pending'
                AND (next_retry_epoch IS NULL OR next_retry_epoch <= ?1))
            OR status = 'processing'",
    )?;
    let rows = stmt.query_map([now], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
        ))
    })?;
    for task in rows {
        match task? {
            (task_id, status, Some(event_id), None)
                if status == "pending" && migrated_tasks.contains(&(task_id, event_id)) => {}
            _ => return Ok(true),
        }
    }
    Ok(false)
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

fn auto_migrate_candidate_with_detector(
    conn: &mut Connection,
    row_id: i64,
    detector: &mut dyn FnMut(&str) -> Option<String>,
    migrated_tasks: &HashSet<(i64, i64)>,
) -> Result<AutoCandidateOutcome> {
    let preflight_now = chrono::Utc::now().timestamp();
    let Some(preflight_row) = load_auto_actionable_row(conn, row_id, preflight_now)? else {
        return Ok(AutoCandidateOutcome::Skipped);
    };
    normalize_capture_host(&preflight_row.legacy.host)?;
    let prepared = prepare_legacy_replay_with_detector(&preflight_row.legacy, detector);
    if current_extraction_work_is_ready(conn, migrated_tasks)? {
        return Ok(AutoCandidateOutcome::YieldedToCurrentWork);
    }

    let mut tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("begin legacy pending auto-migration transaction")?;
    if current_extraction_work_is_ready(&tx, migrated_tasks)? {
        tx.commit()?;
        return Ok(AutoCandidateOutcome::YieldedToCurrentWork);
    }
    let eligibility_now = chrono::Utc::now().timestamp();
    let Some(row) = load_auto_actionable_row(&tx, row_id, eligibility_now)? else {
        tx.commit()?;
        return Ok(AutoCandidateOutcome::Skipped);
    };
    if preflight_row != row || !prepared.matches(&row.legacy) {
        tx.commit()?;
        return Ok(AutoCandidateOutcome::Skipped);
    }
    let host = normalize_capture_host(&row.legacy.host)?;
    let replay = {
        let savepoint = tx
            .savepoint_with_name("legacy_pending_auto_replay")
            .context("begin legacy pending replay savepoint")?;
        let replay = replay_prepared_legacy_row_into_capture(&savepoint, &prepared, host);
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
    let migration = match replay {
        Ok(migration) => migration,
        Err(error) => {
            let retry = mark_legacy_row_for_transient_retry(
                &tx,
                row_id,
                eligibility_now,
                &format!("{error:#}"),
            )
            .with_context(|| format!("record legacy pending retry state id={row_id}"))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "legacy pending row changed inside immediate transaction id={row_id}"
                )
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
    };

    let completed_at = chrono::Utc::now().timestamp();
    let changed = mark_auto_migrated(&tx, row_id, eligibility_now, completed_at)?;
    match changed {
        0 => {
            tx.rollback()?;
            Ok(AutoCandidateOutcome::Skipped)
        }
        1 => {
            tx.commit()?;
            Ok(AutoCandidateOutcome::Migrated {
                extraction_task_id: migration.extraction_task_id,
                captured_event_id: migration.captured_event_id,
            })
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
) -> Result<Option<LegacyPendingSnapshot>> {
    let sql = format!(
        "SELECT {LEGACY_PENDING_SNAPSHOT_COLUMNS}
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
        snapshot_from_db,
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
) -> Result<Vec<LegacyPendingSnapshot>> {
    let limit = limit.max(1);
    let now = chrono::Utc::now().timestamp();
    let sql = format!(
        "SELECT {LEGACY_PENDING_SNAPSHOT_COLUMNS}
         FROM pending_observations
         WHERE (:project IS NULL OR project = :project)
           AND {MANUAL_ELIGIBLE_PREDICATE}
         ORDER BY created_at_epoch ASC, id ASC
         LIMIT :limit"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        named_params! {
            ":project": project,
            ":now": now,
            ":limit": limit,
        },
        snapshot_from_db,
    )?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn load_legacy_pending_row(
    conn: &Connection,
    pending_id: i64,
    now: i64,
) -> Result<Option<LegacyPendingSnapshot>> {
    let sql = format!(
        "SELECT {LEGACY_PENDING_SNAPSHOT_COLUMNS}
         FROM pending_observations
         WHERE id = :id
           AND {MANUAL_ELIGIBLE_PREDICATE}"
    );
    conn.query_row(
        &sql,
        named_params! {
            ":id": pending_id,
            ":now": now,
        },
        snapshot_from_db,
    )
    .optional()
    .map_err(Into::into)
}

fn snapshot_from_db(row: &rusqlite::Row<'_>) -> rusqlite::Result<LegacyPendingSnapshot> {
    Ok(LegacyPendingSnapshot {
        legacy: row_from_db(row)?,
        updated_at_epoch: row.get(9)?,
        status: row.get(10)?,
        attempt_count: row.get(11)?,
        next_retry_epoch: row.get(12)?,
        last_error: row.get(13)?,
        lease_owner: row.get(14)?,
        lease_expires_epoch: row.get(15)?,
        failure_class: row.get(16)?,
        failed_at_epoch: row.get(17)?,
        archived_at_epoch: row.get(18)?,
    })
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

fn legacy_capture_content(row: &LegacyPendingRow, git_branch: Option<&str>) -> String {
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
