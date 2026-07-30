use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;

use super::migration::{
    prepare_legacy_replay_with_detector, replay_prepared_legacy_row_into_capture,
    LegacyPendingMigration, LegacyPendingRow,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchivedLegacyPendingRecoveryPreview {
    pub pending_id: i64,
    pub stored_host: String,
    pub resolved_host: String,
    pub requires_host: bool,
    pub project: String,
    pub session_id: String,
    pub failure_class: Option<String>,
    pub archived_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArchivedLegacyPendingRecovery {
    pub candidate: ArchivedLegacyPendingRecoveryPreview,
    pub migrated: LegacyPendingMigration,
}

#[derive(Clone, PartialEq, Eq)]
struct ArchivedLegacyPendingRow {
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
    archived_at_epoch: i64,
}

pub fn preview_archived_legacy_pending_recovery(
    conn: &Connection,
    pending_id: i64,
    fallback_host: Option<&str>,
) -> Result<ArchivedLegacyPendingRecoveryPreview> {
    ensure_positive_id(pending_id)?;
    let fallback_host = validate_fallback_host(fallback_host)?;
    let row = load_archived_failed_row(conn, pending_id)?;
    preview_for_row(&row, fallback_host)
}

pub fn recover_archived_legacy_pending(
    conn: &mut Connection,
    pending_id: i64,
    fallback_host: Option<&str>,
) -> Result<ArchivedLegacyPendingRecovery> {
    let mut detector = crate::db::detect_git_branch;
    recover_archived_legacy_pending_with_detector(conn, pending_id, fallback_host, &mut detector)
}

fn recover_archived_legacy_pending_with_detector(
    conn: &mut Connection,
    pending_id: i64,
    fallback_host: Option<&str>,
    detector: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<ArchivedLegacyPendingRecovery> {
    ensure_positive_id(pending_id)?;
    let fallback_host = validate_fallback_host(fallback_host)?;
    let preflight_row = load_archived_failed_row(conn, pending_id)?;
    preview_for_row(&preflight_row, fallback_host)?;
    let prepared = prepare_legacy_replay_with_detector(&preflight_row.legacy, detector);

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("begin archived legacy pending recovery transaction")?;
    let row = load_archived_failed_row(&tx, pending_id)?;
    if row != preflight_row || !prepared.matches(&row.legacy) {
        bail!(
            "archived legacy pending row {pending_id} changed while preparing recovery; retry the exact command"
        );
    }
    let candidate = preview_for_row(&row, fallback_host)?;
    let migrated =
        replay_prepared_legacy_row_into_capture(&tx, &prepared, &candidate.resolved_host)
            .with_context(|| format!("replay archived legacy pending row {pending_id}"))?;
    let completed_at = chrono::Utc::now().timestamp();
    let changed = tx.execute(
        "UPDATE pending_observations
         SET status = 'migrated',
             attempt_count = 0,
             next_retry_epoch = NULL,
             last_error = NULL,
             lease_owner = NULL,
             lease_expires_epoch = NULL,
             failure_class = NULL,
             failed_at_epoch = NULL,
             archived_at_epoch = NULL,
             updated_at_epoch = ?2
         WHERE id = ?1
           AND status = 'failed'
           AND archived_at_epoch IS NOT NULL",
        params![pending_id, completed_at],
    )?;
    if changed != 1 {
        bail!(
            "archived legacy pending row {pending_id} changed while recovering; replay was rolled back"
        );
    }
    tx.commit()
        .context("commit archived legacy pending recovery")?;
    Ok(ArchivedLegacyPendingRecovery {
        candidate,
        migrated,
    })
}

fn ensure_positive_id(pending_id: i64) -> Result<()> {
    if pending_id <= 0 {
        bail!("archived legacy pending recovery requires a positive --id");
    }
    Ok(())
}

fn validate_fallback_host(host: Option<&str>) -> Result<Option<&str>> {
    match host {
        None => Ok(None),
        Some(host @ (crate::runtime_config::CLAUDE_HOST | crate::runtime_config::CODEX_HOST)) => {
            Ok(Some(host))
        }
        Some(host) => bail!("invalid recovery host '{host}'; expected claude-code or codex-cli"),
    }
}

fn preview_for_row(
    row: &ArchivedLegacyPendingRow,
    fallback_host: Option<&str>,
) -> Result<ArchivedLegacyPendingRecoveryPreview> {
    let stored_host_is_known = matches!(
        row.legacy.host.as_str(),
        crate::runtime_config::CLAUDE_HOST | crate::runtime_config::CODEX_HOST
    );
    let resolved_host = if stored_host_is_known {
        row.legacy.host.clone()
    } else {
        fallback_host
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "archived legacy pending row {} has host='{}'; pass --host claude-code or --host codex-cli",
                    row.legacy.id,
                    row.legacy.host
                )
            })?
            .to_string()
    };
    Ok(ArchivedLegacyPendingRecoveryPreview {
        pending_id: row.legacy.id,
        stored_host: row.legacy.host.clone(),
        resolved_host,
        requires_host: !stored_host_is_known,
        project: row.legacy.project.clone(),
        session_id: row.legacy.session_id.clone(),
        failure_class: row.failure_class.clone(),
        archived_at_epoch: row.archived_at_epoch,
    })
}

fn load_archived_failed_row(
    conn: &Connection,
    pending_id: i64,
) -> Result<ArchivedLegacyPendingRow> {
    conn.query_row(
        "SELECT id, host, session_id, project, tool_name, tool_input, tool_response, cwd,
                created_at_epoch, updated_at_epoch, status, attempt_count, next_retry_epoch,
                last_error, lease_owner, lease_expires_epoch, failure_class, failed_at_epoch,
                archived_at_epoch
         FROM pending_observations
         WHERE id = ?1
           AND status = 'failed'
           AND archived_at_epoch IS NOT NULL",
        [pending_id],
        |row| {
            Ok(ArchivedLegacyPendingRow {
                legacy: LegacyPendingRow {
                    id: row.get(0)?,
                    host: row.get(1)?,
                    session_id: row.get(2)?,
                    project: row.get(3)?,
                    tool_name: row.get(4)?,
                    tool_input: row.get(5)?,
                    tool_response: row.get(6)?,
                    cwd: row.get(7)?,
                    created_at_epoch: row.get(8)?,
                },
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
        },
    )
    .optional()?
    .ok_or_else(|| {
        anyhow::anyhow!(
            "archived legacy pending row {pending_id} is not recoverable: expected status='failed' with archived_at_epoch set"
        )
    })
}

#[cfg(test)]
mod tests;
