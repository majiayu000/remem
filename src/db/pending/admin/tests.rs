use rusqlite::{params, Connection};

use super::{
    count_failed_purge_candidates, count_failed_retry_candidates,
    count_legacy_migration_candidates, list_failed, migrate_legacy_pending, purge_failed,
    retry_failed,
};
use crate::{db, migrate::MIGRATIONS};

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    for migration in MIGRATIONS {
        conn.execute_batch(migration.sql)
            .expect("schema migration should load");
    }
    conn
}

fn insert_failed_row(
    conn: &Connection,
    session_id: &str,
    project: &str,
    updated_at_epoch: i64,
    last_error: &str,
) -> i64 {
    let id = db::enqueue_pending(
        conn,
        "codex-cli",
        session_id,
        project,
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row should enqueue");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'failed',
             attempt_count = 3,
             updated_at_epoch = ?2,
             last_error = ?3,
             lease_owner = 'worker-x',
             lease_expires_epoch = ?2,
             next_retry_epoch = ?2
         WHERE id = ?1",
        params![id, updated_at_epoch, last_error],
    )
    .expect("failed row should update");
    id
}

#[test]
fn list_failed_filters_by_project_and_limit() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let newest = insert_failed_row(&conn, "s-1", "alpha", now - 10, "err-1");
    insert_failed_row(&conn, "s-2", "alpha", now - 20, "err-2");
    insert_failed_row(&conn, "s-3", "beta", now - 5, "err-3");

    let rows = list_failed(&conn, Some("alpha"), 1).expect("failed rows should load");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, newest);
    assert_eq!(rows[0].project, "alpha");
}

#[test]
fn retry_failed_resets_rows_for_selected_project() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let alpha_id = insert_failed_row(&conn, "s-1", "alpha", now - 10, "alpha boom");
    let beta_id = insert_failed_row(&conn, "s-2", "beta", now - 20, "beta boom");

    let changed = retry_failed(&conn, Some("alpha"), 5).expect("retry should succeed");

    assert_eq!(changed, 1);
    let alpha_row = conn
        .query_row(
            "SELECT status, lease_owner, lease_expires_epoch, next_retry_epoch, last_error
             FROM pending_observations WHERE id = ?1",
            params![alpha_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .expect("alpha row should exist");
    assert_eq!(alpha_row.0, "pending");
    assert_eq!(alpha_row.1, None);
    assert_eq!(alpha_row.2, None);
    assert_eq!(alpha_row.3, None);
    assert_eq!(alpha_row.4, None);

    let beta_status: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            params![beta_id],
            |row| row.get(0),
        )
        .expect("beta row should exist");
    assert_eq!(beta_status, "failed");
}

#[test]
fn retry_failed_skips_archived_rows() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let archived_id = insert_failed_row(&conn, "s-1", "alpha", now - 5, "archived boom");
    let active_id = insert_failed_row(&conn, "s-2", "alpha", now - 10, "active boom");
    conn.execute(
        "UPDATE pending_observations
         SET archived_at_epoch = ?1
         WHERE id = ?2",
        params![now - 1, archived_id],
    )
    .expect("archived row should update");

    let changed = retry_failed(&conn, None, 1).expect("retry should succeed");

    assert_eq!(changed, 1);
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT id, status FROM pending_observations ORDER BY id ASC")
        .expect("select should prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("rows should query")
        .collect::<Result<_, _>>()
        .expect("rows should collect");
    assert!(rows.contains(&(archived_id, "failed".to_string())));
    assert!(rows.contains(&(active_id, "pending".to_string())));
}

#[test]
fn retry_failed_dry_run_count_respects_project_and_limit_without_mutation() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let alpha_id = insert_failed_row(&conn, "s-1", "alpha", now - 10, "alpha newest");
    insert_failed_row(&conn, "s-2", "alpha", now - 20, "alpha older");
    insert_failed_row(&conn, "s-3", "beta", now - 5, "beta newest");

    let count =
        count_failed_retry_candidates(&conn, Some("alpha"), 1).expect("dry-run count should query");

    assert_eq!(count, 1);
    let status: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            params![alpha_id],
            |row| row.get(0),
        )
        .expect("alpha row should exist");
    assert_eq!(status, "failed");
}

#[test]
fn retry_failed_dry_run_count_skips_archived_rows_before_limit() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let archived_id = insert_failed_row(&conn, "s-1", "alpha", now - 5, "archived newest");
    insert_failed_row(&conn, "s-2", "alpha", now - 10, "active older");
    conn.execute(
        "UPDATE pending_observations
         SET archived_at_epoch = ?1
         WHERE id = ?2",
        params![now - 1, archived_id],
    )
    .expect("archived row should update");

    let count =
        count_failed_retry_candidates(&conn, Some("alpha"), 1).expect("dry-run count should query");

    assert_eq!(count, 1);
}

#[test]
fn purge_failed_respects_cutoff_and_project() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let old_alpha = insert_failed_row(&conn, "s-1", "alpha", now - 5 * 86_400, "old alpha");
    let recent_alpha = insert_failed_row(&conn, "s-2", "alpha", now - 86_400, "recent alpha");
    let old_beta = insert_failed_row(&conn, "s-3", "beta", now - 5 * 86_400, "old beta");

    let changed = purge_failed(&conn, Some("alpha"), 2).expect("purge should succeed");

    assert_eq!(changed, 1);
    let remaining_ids = conn
        .prepare("SELECT id FROM pending_observations ORDER BY id ASC")
        .expect("select should prepare")
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("rows should load")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("rows should collect");
    assert_eq!(remaining_ids, vec![recent_alpha, old_beta]);
    assert!(!remaining_ids.contains(&old_alpha));
}

#[test]
fn purge_failed_dry_run_count_respects_cutoff_without_deleting() {
    let conn = setup_conn();
    let now = chrono::Utc::now().timestamp();
    let old_alpha = insert_failed_row(&conn, "s-1", "alpha", now - 5 * 86_400, "old alpha");
    insert_failed_row(&conn, "s-2", "alpha", now - 86_400, "recent alpha");
    insert_failed_row(&conn, "s-3", "beta", now - 5 * 86_400, "old beta");

    let count =
        count_failed_purge_candidates(&conn, Some("alpha"), 2).expect("dry-run count should query");

    assert_eq!(count, 1);
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pending_observations", [], |row| {
            row.get(0)
        })
        .expect("row count should query");
    assert_eq!(row_count, 3);
    let status: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            params![old_alpha],
            |row| row.get(0),
        )
        .expect("old alpha should remain");
    assert_eq!(status, "failed");
}

#[test]
fn migrate_legacy_pending_replays_rows_into_capture_pipeline() {
    let mut conn = setup_conn();
    let id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "sess-legacy",
        "alpha",
        "Edit",
        Some(r#"{"file_path":"src/lib.rs"}"#),
        Some("edited"),
        Some("/tmp/remem"),
    )
    .expect("legacy row should enqueue");

    let migrated = migrate_legacy_pending(&mut conn, Some("alpha"), None, 10)
        .expect("legacy migration should succeed");

    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].pending_id, id);
    assert_eq!(migrated[0].event_id, format!("legacy-pending-{id}"));
    assert_eq!(migrated[0].host, "codex-cli");
    let status: String = conn
        .query_row(
            "SELECT status FROM pending_observations WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("legacy row should remain auditable");
    assert_eq!(status, "migrated");
    let (captured, tasks): (i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM captured_events),
                (SELECT COUNT(*) FROM extraction_tasks)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("capture counts should query");
    assert_eq!(captured, 1);
    assert_eq!(tasks, 1);
    let captured_created_at: i64 = conn
        .query_row("SELECT created_at_epoch FROM captured_events", [], |row| {
            row.get(0)
        })
        .expect("captured timestamp should query");
    let legacy_created_at: i64 = conn
        .query_row(
            "SELECT created_at_epoch FROM pending_observations WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("legacy timestamp should query");
    assert_eq!(captured_created_at, legacy_created_at);
}

#[test]
fn migrate_legacy_pending_requires_host_for_unknown_rows() {
    let mut conn = setup_conn();
    let id = db::enqueue_pending(
        &conn,
        "unknown",
        "sess-legacy",
        "alpha",
        "Edit",
        Some(r#"{"file_path":"src/lib.rs"}"#),
        None,
        Some("/tmp/remem"),
    )
    .expect("legacy row should enqueue");

    let error = migrate_legacy_pending(&mut conn, Some("alpha"), None, 10)
        .expect_err("unknown legacy host should fail closed");

    assert!(error.to_string().contains("--host"));
    let (status, captured): (String, i64) = conn
        .query_row(
            "SELECT
                (SELECT status FROM pending_observations WHERE id = ?1),
                (SELECT COUNT(*) FROM captured_events)",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("rows should query");
    assert_eq!(status, "pending");
    assert_eq!(captured, 0);
}

#[test]
fn migrate_legacy_pending_uses_fallback_host_and_is_idempotent() {
    let mut conn = setup_conn();
    let id = db::enqueue_pending(
        &conn,
        "unknown",
        "sess-legacy",
        "alpha",
        "Bash",
        Some(r#"{"command":"cargo test"}"#),
        Some(r#"{"exitCode":0}"#),
        Some("/tmp/remem"),
    )
    .expect("legacy row should enqueue");

    assert_eq!(
        count_legacy_migration_candidates(&conn, Some("alpha"), 10)
            .expect("dry run count should query"),
        1
    );
    let migrated = migrate_legacy_pending(&mut conn, Some("alpha"), Some("claude-code"), 10)
        .expect("fallback host migration should succeed");
    let second = migrate_legacy_pending(&mut conn, Some("alpha"), Some("claude-code"), 10)
        .expect("second migration should be a no-op");

    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].pending_id, id);
    assert_eq!(migrated[0].host, "claude-code");
    assert!(second.is_empty());
    assert_eq!(
        count_legacy_migration_candidates(&conn, Some("alpha"), 10)
            .expect("dry run count should query"),
        0
    );
    let captured: i64 = conn
        .query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))
        .expect("capture count should query");
    assert_eq!(captured, 1);
}

#[test]
fn migrate_legacy_pending_dry_run_counts_expired_processing_rows() {
    let conn = setup_conn();
    let expired_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "sess-expired",
        "alpha",
        "Edit",
        Some(r#"{"file_path":"src/lib.rs"}"#),
        None,
        Some("/tmp/remem"),
    )
    .expect("expired row should enqueue");
    let active_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "sess-active",
        "alpha",
        "Edit",
        Some(r#"{"file_path":"src/main.rs"}"#),
        None,
        Some("/tmp/remem"),
    )
    .expect("active row should enqueue");
    let beta_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "sess-beta",
        "beta",
        "Edit",
        Some(r#"{"file_path":"src/db.rs"}"#),
        None,
        Some("/tmp/remem"),
    )
    .expect("beta row should enqueue");
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'old-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![expired_id, now - 1],
    )
    .expect("expired row should update");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'live-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![active_id, now + 300],
    )
    .expect("active row should update");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'old-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![beta_id, now - 1],
    )
    .expect("beta row should update");

    assert_eq!(
        count_legacy_migration_candidates(&conn, Some("alpha"), 10)
            .expect("alpha dry-run count should query"),
        1
    );
    assert_eq!(
        count_legacy_migration_candidates(&conn, None, 10)
            .expect("global dry-run count should query"),
        2
    );
    let statuses = conn
        .prepare("SELECT id, status FROM pending_observations ORDER BY id ASC")
        .expect("status select should prepare")
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .expect("status rows should query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("status rows should collect");
    assert_eq!(
        statuses,
        vec![
            (expired_id, "processing".to_string()),
            (active_id, "processing".to_string()),
            (beta_id, "processing".to_string())
        ]
    );
}

#[test]
fn migrate_legacy_pending_replays_expired_processing_rows() {
    let mut conn = setup_conn();
    let expired_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "sess-expired",
        "alpha",
        "Edit",
        Some(r#"{"file_path":"src/lib.rs"}"#),
        None,
        Some("/tmp/remem"),
    )
    .expect("expired row should enqueue");
    let active_id = db::enqueue_pending(
        &conn,
        "codex-cli",
        "sess-active",
        "alpha",
        "Edit",
        Some(r#"{"file_path":"src/main.rs"}"#),
        None,
        Some("/tmp/remem"),
    )
    .expect("active row should enqueue");
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'old-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![expired_id, now - 1],
    )
    .expect("expired row should update");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_owner = 'live-worker', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![active_id, now + 300],
    )
    .expect("active row should update");

    let migrated = migrate_legacy_pending(&mut conn, Some("alpha"), None, 10)
        .expect("expired processing row should migrate");

    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].pending_id, expired_id);
    let statuses = conn
        .prepare("SELECT id, status FROM pending_observations ORDER BY id ASC")
        .expect("status select should prepare")
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .expect("status rows should query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("status rows should collect");
    assert_eq!(
        statuses,
        vec![
            (expired_id, "migrated".to_string()),
            (active_id, "processing".to_string())
        ]
    );
}
