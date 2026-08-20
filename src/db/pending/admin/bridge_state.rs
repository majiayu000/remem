use anyhow::{Context, Result};
use rusqlite::{named_params, Connection, OptionalExtension};

use crate::runtime_config::{CLAUDE_HOST, CODEX_HOST};

pub const LEGACY_PENDING_REMOVAL_VERSION: &str = "0.7.0";
pub const LEGACY_PENDING_SURFACE: &str = "pending_observations";
pub const LEGACY_PENDING_STATE_EXHAUSTED: &str = "exhausted";
pub const LEGACY_PENDING_STATE_DRAINING: &str = "frozen_draining";

pub(super) const AUTO_ACTIONABLE_PREDICATE: &str = "
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

const AUTO_RECOVERABLE_PREDICATE: &str = "
    host IN (:claude_host, :codex_host)
    AND (
        status = 'pending'
        OR status = 'processing'
        OR (status = 'failed'
            AND COALESCE(failure_class, 'transient') = 'transient')
    )";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyPendingBridgeState {
    FrozenDraining,
    Exhausted,
}

impl LegacyPendingBridgeState {
    fn as_str(self) -> &'static str {
        match self {
            Self::FrozenDraining => LEGACY_PENDING_STATE_DRAINING,
            Self::Exhausted => LEGACY_PENDING_STATE_EXHAUSTED,
        }
    }
}

pub fn has_auto_actionable_legacy_pending(conn: &Connection) -> Result<bool> {
    Ok(count_auto_actionable_legacy_pending(conn)? > 0)
}

pub fn count_auto_actionable_legacy_pending(conn: &Connection) -> Result<i64> {
    if !legacy_surface_state_ready(conn)? && !pending_observations_ready(conn)? {
        return Ok(0);
    }
    let now = chrono::Utc::now().timestamp();
    let sql =
        format!("SELECT COUNT(*) FROM pending_observations WHERE {AUTO_ACTIONABLE_PREDICATE}");
    conn.query_row(
        &sql,
        named_params! {
            ":now": now,
            ":claude_host": CLAUDE_HOST,
            ":codex_host": CODEX_HOST,
        },
        |row| row.get(0),
    )
    .context("count auto-actionable legacy pending rows")
}

fn count_auto_recoverable_legacy_pending(conn: &Connection) -> Result<i64> {
    if !legacy_surface_state_ready(conn)? && !pending_observations_ready(conn)? {
        return Ok(0);
    }
    let sql =
        format!("SELECT COUNT(*) FROM pending_observations WHERE {AUTO_RECOVERABLE_PREDICATE}");
    conn.query_row(
        &sql,
        named_params! {
            ":claude_host": CLAUDE_HOST,
            ":codex_host": CODEX_HOST,
        },
        |row| row.get(0),
    )
    .context("count auto-recoverable legacy pending rows")
}

pub fn legacy_pending_auto_bridge_is_exhausted(conn: &Connection) -> Result<bool> {
    let Some(state) = load_legacy_pending_bridge_state(conn)? else {
        return Ok(false);
    };
    Ok(state == LegacyPendingBridgeState::Exhausted)
}

pub fn sync_legacy_pending_bridge_state(conn: &Connection) -> Result<LegacyPendingBridgeState> {
    let residual = count_auto_recoverable_legacy_pending(conn)?;
    let state = if residual == 0 {
        LegacyPendingBridgeState::Exhausted
    } else {
        LegacyPendingBridgeState::FrozenDraining
    };
    write_legacy_pending_bridge_state(conn, state, residual)?;
    Ok(state)
}

pub fn reactivate_legacy_pending_bridge(conn: &Connection) -> Result<()> {
    if !legacy_surface_state_ready(conn)? {
        return Ok(());
    }
    let residual = count_auto_recoverable_legacy_pending(conn)?;
    if residual == 0 {
        return Ok(());
    }
    write_legacy_pending_bridge_state(conn, LegacyPendingBridgeState::FrozenDraining, residual)
}

fn load_legacy_pending_bridge_state(conn: &Connection) -> Result<Option<LegacyPendingBridgeState>> {
    if !legacy_surface_state_ready(conn)? {
        return Ok(None);
    }
    let raw: Option<String> = conn
        .query_row(
            "SELECT state FROM legacy_surface_state WHERE surface = ?1",
            [LEGACY_PENDING_SURFACE],
            |row| row.get(0),
        )
        .optional()
        .context("load legacy pending bridge state")?;
    match raw.as_deref() {
        Some(LEGACY_PENDING_STATE_EXHAUSTED) => Ok(Some(LegacyPendingBridgeState::Exhausted)),
        Some(LEGACY_PENDING_STATE_DRAINING) => Ok(Some(LegacyPendingBridgeState::FrozenDraining)),
        Some(other) => anyhow::bail!("unknown legacy_surface_state.state={other}"),
        None => Ok(None),
    }
}

fn write_legacy_pending_bridge_state(
    conn: &Connection,
    state: LegacyPendingBridgeState,
    residual_count: i64,
) -> Result<()> {
    if !legacy_surface_state_ready(conn)? {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    let exhausted_at = match state {
        LegacyPendingBridgeState::Exhausted => Some(now),
        LegacyPendingBridgeState::FrozenDraining => None,
    };
    conn.execute(
        "INSERT INTO legacy_surface_state (
             surface, state, residual_count, exhausted_at_epoch, updated_at_epoch
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(surface) DO UPDATE SET
             state = excluded.state,
             residual_count = excluded.residual_count,
             exhausted_at_epoch = excluded.exhausted_at_epoch,
             updated_at_epoch = excluded.updated_at_epoch",
        rusqlite::params![
            LEGACY_PENDING_SURFACE,
            state.as_str(),
            residual_count,
            exhausted_at,
            now
        ],
    )
    .context("persist legacy pending bridge state")?;
    Ok(())
}

fn legacy_surface_state_ready(conn: &Connection) -> Result<bool> {
    table_exists(conn, "legacy_surface_state")
}

fn pending_observations_ready(conn: &Connection) -> Result<bool> {
    table_exists(conn, "pending_observations")
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()
        .with_context(|| format!("inspect sqlite_master for {name}"))?;
    Ok(exists.is_some())
}

#[cfg(test)]
mod tests;
