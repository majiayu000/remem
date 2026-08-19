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
