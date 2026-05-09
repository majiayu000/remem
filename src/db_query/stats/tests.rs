use rusqlite::Connection;

use super::{DailyActivityStats, ProjectCount, SystemStats};
use crate::db_query::{query_daily_activity_stats, query_system_stats, query_top_projects};

fn setup_stats_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE session_summaries (
            id INTEGER PRIMARY KEY
        );
        CREATE TABLE raw_messages (
            id INTEGER PRIMARY KEY
        );
        CREATE TABLE pending_observations (
            id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL DEFAULT 0,
            next_retry_epoch INTEGER,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        );
        CREATE TABLE jobs (
            id INTEGER PRIMARY KEY,
            state TEXT NOT NULL,
            lease_expires_epoch INTEGER
        );
        CREATE TABLE worker_heartbeats (
            owner TEXT PRIMARY KEY,
            pid INTEGER,
            started_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );",
    )
    .expect("schema should be created");
}

#[test]
fn query_system_stats_and_related_views_share_one_definition() {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    setup_stats_schema(&conn);

    conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch) VALUES ('alpha', 'active', 200)",
        [],
    )
    .expect("active memory insert should succeed");
    conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch) VALUES ('alpha', 'archived', 150)",
        [],
    )
    .expect("archived memory insert should succeed");
    conn.execute(
        "INSERT INTO memories (project, status, created_at_epoch) VALUES ('beta', 'active', 300)",
        [],
    )
    .expect("second active memory insert should succeed");
    conn.execute(
        "INSERT INTO observations (project, status, created_at_epoch) VALUES ('alpha', 'active', 220)",
        [],
    )
    .expect("active observation insert should succeed");
    conn.execute(
        "INSERT INTO observations (project, status, created_at_epoch) VALUES ('beta', 'stale', 140)",
        [],
    )
    .expect("stale observation insert should succeed");
    conn.execute("INSERT INTO session_summaries (id) VALUES (1)", [])
        .expect("summary insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch) VALUES ('pending', 100)",
        [],
    )
    .expect("pending insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch) VALUES ('pending', 120)",
        [],
    )
    .expect("second pending insert should succeed");
    conn.execute(
        "UPDATE pending_observations SET next_retry_epoch = strftime('%s', 'now') + 3600 WHERE id = 2",
        [],
    )
    .expect("delayed pending update should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch, lease_owner, lease_expires_epoch)
         VALUES ('processing', 130, 'worker-a', strftime('%s', 'now') - 1)",
        [],
    )
    .expect("processing pending insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status, created_at_epoch) VALUES ('failed', 140)",
        [],
    )
    .expect("failed pending insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('pending', NULL)",
        [],
    )
    .expect("pending job insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('processing', 0)",
        [],
    )
    .expect("stuck job insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('failed', NULL)",
        [],
    )
    .expect("failed job insert should succeed");
    conn.execute(
        "INSERT INTO worker_heartbeats (owner, pid, started_at_epoch, updated_at_epoch)
         VALUES ('worker-a', 123, strftime('%s', 'now') - 10, strftime('%s', 'now') - 10)",
        [],
    )
    .expect("heartbeat insert should succeed");

    let system = query_system_stats(&conn).expect("system stats should load");
    assert_eq!(
        system,
        SystemStats {
            active_memories: 2,
            active_observations: 1,
            session_summaries: 1,
            raw_messages: 0,
            pending_observations: 2,
            ready_pending_observations: 1,
            delayed_pending_observations: 1,
            processing_pending_observations: 1,
            expired_processing_pending_observations: 1,
            failed_pending_observations: 1,
            oldest_ready_pending_epoch: Some(100),
            pending_jobs: 1,
            processing_jobs: 1,
            failed_jobs: 1,
            stuck_jobs: 1,
            worker_daemon_healthy: true,
            worker_heartbeat_owner: Some("worker-a".to_string()),
            worker_heartbeat_age_secs: system.worker_heartbeat_age_secs,
        }
    );
    assert!(
        system.worker_heartbeat_age_secs.unwrap_or_default() <= 20,
        "heartbeat age should be recent"
    );

    let daily = query_daily_activity_stats(&conn, 180).expect("daily stats should load");
    assert_eq!(
        daily,
        DailyActivityStats {
            memories: 2,
            observations: 1,
        }
    );

    let top_projects = query_top_projects(&conn, 5).expect("top projects should load");
    assert_eq!(
        top_projects,
        vec![
            ProjectCount {
                project: "alpha".to_string(),
                count: 1,
            },
            ProjectCount {
                project: "beta".to_string(),
                count: 1,
            },
        ]
    );
}
