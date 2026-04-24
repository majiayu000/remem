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
            status TEXT NOT NULL
        );
        CREATE TABLE jobs (
            id INTEGER PRIMARY KEY,
            state TEXT NOT NULL,
            lease_expires_epoch INTEGER
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
        "INSERT INTO pending_observations (status) VALUES ('pending')",
        [],
    )
    .expect("pending insert should succeed");
    conn.execute(
        "INSERT INTO pending_observations (status) VALUES ('failed')",
        [],
    )
    .expect("failed pending insert should succeed");
    conn.execute(
        "INSERT INTO jobs (state, lease_expires_epoch) VALUES ('running', 0)",
        [],
    )
    .expect("stuck job insert should succeed");

    let system = query_system_stats(&conn).expect("system stats should load");
    assert_eq!(
        system,
        SystemStats {
            active_memories: 2,
            active_observations: 1,
            session_summaries: 1,
            raw_messages: 0,
            pending_observations: 1,
            failed_pending_observations: 1,
            stuck_jobs: 1,
        }
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
