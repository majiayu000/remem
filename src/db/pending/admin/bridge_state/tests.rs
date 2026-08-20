use super::*;
use crate::runtime_config::CODEX_HOST;
use rusqlite::Connection;

fn setup_conn() -> anyhow::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[test]
fn fresh_store_persists_exhausted_bridge_state() -> anyhow::Result<()> {
    let conn = setup_conn()?;

    assert!(legacy_pending_auto_bridge_is_exhausted(&conn)?);
    assert!(!has_auto_actionable_legacy_pending(&conn)?);
    assert_eq!(
        load_legacy_pending_bridge_state(&conn)?,
        Some(LegacyPendingBridgeState::Exhausted)
    );
    Ok(())
}

#[test]
fn fixture_reactivates_and_sync_exhausts_after_clear() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        CODEX_HOST,
        "sess-bridge",
        "/tmp/remem-bridge",
        "Bash",
        None,
        None,
        None,
    )?;

    assert!(!legacy_pending_auto_bridge_is_exhausted(&conn)?);
    assert!(has_auto_actionable_legacy_pending(&conn)?);

    conn.execute("UPDATE pending_observations SET status = 'migrated'", [])?;
    assert_eq!(
        sync_legacy_pending_bridge_state(&conn)?,
        LegacyPendingBridgeState::Exhausted
    );
    assert!(legacy_pending_auto_bridge_is_exhausted(&conn)?);
    Ok(())
}

#[test]
fn delayed_and_leased_rows_keep_bridge_draining() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let delayed = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        CODEX_HOST,
        "sess-delayed",
        "/tmp/remem-delayed",
        "Bash",
        None,
        None,
        None,
    )?;
    let leased = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        CODEX_HOST,
        "sess-leased",
        "/tmp/remem-leased",
        "Bash",
        None,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed', failure_class = 'transient', next_retry_epoch = ?2
         WHERE id = ?1",
        rusqlite::params![delayed, now + 300],
    )?;
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'live', lease_expires_epoch = ?2
         WHERE id = ?1",
        rusqlite::params![leased, now + 300],
    )?;

    assert!(!has_auto_actionable_legacy_pending(&conn)?);
    assert_eq!(
        sync_legacy_pending_bridge_state(&conn)?,
        LegacyPendingBridgeState::FrozenDraining
    );
    assert!(!legacy_pending_auto_bridge_is_exhausted(&conn)?);
    Ok(())
}

#[test]
fn retry_failed_reactivates_exhausted_auto_bridge() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let id = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        CODEX_HOST,
        "sess-retry-reactivate",
        "alpha",
        "Bash",
        None,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed', failure_class = 'transient',
             next_retry_epoch = ?2, failed_at_epoch = ?2
         WHERE id = ?1",
        rusqlite::params![id, now - 10],
    )?;
    conn.execute(
        "UPDATE legacy_surface_state
         SET state = 'exhausted', residual_count = 0, exhausted_at_epoch = ?1
         WHERE surface = 'pending_observations'",
        [now],
    )?;

    let changed = super::super::mutate::retry_failed(&conn, Some("alpha"), 5)?;

    assert_eq!(changed, 1);
    assert!(!legacy_pending_auto_bridge_is_exhausted(&conn)?);
    Ok(())
}
