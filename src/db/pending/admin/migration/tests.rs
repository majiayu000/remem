use super::*;

#[test]
fn unknown_host_count_is_queryable() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE pending_observations (
            id INTEGER PRIMARY KEY,
            host TEXT NOT NULL,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT,
            tool_response TEXT,
            cwd TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL,
            attempt_count INTEGER NOT NULL,
            next_retry_epoch INTEGER,
            last_error TEXT,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        );",
    )?;
    conn.execute(
        "INSERT INTO pending_observations
         (host, session_id, project, tool_name, created_at_epoch, updated_at_epoch, status, attempt_count)
         VALUES ('unknown', 's', 'p', 'Edit', 1, 1, 'pending', 0)",
        [],
    )?;

    assert_eq!(count_legacy_migration_candidates(&conn, Some("p"), 10)?, 1);
    assert_eq!(
        count_legacy_migration_candidates(&conn, Some("other"), 10)?,
        0
    );
    Ok(())
}

#[test]
fn legacy_event_id_is_stable() {
    assert_eq!(legacy_event_id(42), "legacy-pending-42");
}

#[test]
fn retry_backoff_caps_without_rewriting_failure_or_archive_time() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let failed_at = now - 1_000;
    let archived_at = now - 500;
    let id = crate::db::test_support::insert_legacy_pending_fixture(
        &conn,
        crate::runtime_config::CODEX_HOST,
        "s-capped-retry",
        "alpha",
        "tool",
        None,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed', failure_class = 'transient', attempt_count = 20,
             next_retry_epoch = ?2, failed_at_epoch = ?3, archived_at_epoch = ?4
         WHERE id = ?1",
        params![id, now - 1, failed_at, archived_at],
    )?;

    let retry = mark_legacy_row_for_transient_retry(&conn, id, now, "shared failure")?
        .ok_or_else(|| anyhow::anyhow!("retry transition should update the row"))?;

    assert_eq!(retry.attempt_count, 21);
    assert_eq!(retry.backoff_secs, AUTO_MIGRATION_RETRY_MAX_SECS);
    let state: (i64, i64, i64) = conn.query_row(
        "SELECT failed_at_epoch, archived_at_epoch, next_retry_epoch
         FROM pending_observations WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        state,
        (failed_at, archived_at, now + AUTO_MIGRATION_RETRY_MAX_SECS)
    );
    Ok(())
}

#[test]
fn archived_counts_separate_auto_recovery_from_admin_recovery() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let insert_archived = |host: &str, failure_class: &str| -> Result<()> {
        let id = crate::db::test_support::insert_legacy_pending_fixture(
            &conn, host, "s", "alpha", "tool", None, None, None,
        )?;
        conn.execute(
            "UPDATE pending_observations
             SET status = 'failed', failure_class = ?2, archived_at_epoch = ?3
             WHERE id = ?1",
            params![id, failure_class, now - 1],
        )?;
        Ok(())
    };
    insert_archived(crate::runtime_config::CODEX_HOST, "transient")?;
    insert_archived(crate::runtime_config::CODEX_HOST, "permanent")?;
    insert_archived("unknown", "transient")?;

    assert_eq!(count_recoverable_archived_legacy_pending(&conn)?, 1);
    assert_eq!(count_admin_required_archived_legacy_pending(&conn)?, 2);
    Ok(())
}
