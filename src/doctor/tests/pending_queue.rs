use rusqlite::params;

use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::super::database::check_pending_queue;

#[test]
fn check_pending_queue_reports_expired_legacy_pending_as_replay_required() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-expired-legacy-pending");
    let conn = db::open_db().expect("db should open");
    let pending_id = db::test_support::insert_legacy_pending_fixture(
        &conn,
        "codex-cli",
        "session-expired",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row insert should succeed");
    conn.execute(
        "UPDATE pending_observations
         SET status = 'processing', lease_expires_epoch = ?2
         WHERE id = ?1",
        params![pending_id, chrono::Utc::now().timestamp() - 1],
    )?;

    let check = check_pending_queue(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("requires legacy replay"),
        "{}",
        check.detail
    );
    assert!(
        !check.detail.contains("will auto-recover"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("preview replay: `remem pending migrate-legacy --dry-run`"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("apply replay: `remem pending migrate-legacy`"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains(
            "apply replay for Claude host: `remem pending migrate-legacy --host claude-code`"
        ),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains(
            "apply replay for Codex host: `remem pending migrate-legacy --host codex-cli`"
        ),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn check_pending_queue_reports_ready_legacy_pending_as_replay_required() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-ready-legacy-pending");
    let conn = db::open_db().expect("db should open");
    db::test_support::insert_legacy_pending_fixture(
        &conn,
        "codex-cli",
        "session-ready",
        "proj-a",
        "tool",
        None,
        None,
        None,
    )
    .expect("pending row insert should succeed");

    let check = check_pending_queue(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("requires legacy replay"),
        "{}",
        check.detail
    );
    assert!(
        !check.detail.contains("will auto-recover"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("inspect counts: `remem status --json`"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("apply replay: `remem pending migrate-legacy`"),
        "{}",
        check.detail
    );
    Ok(())
}
