use rusqlite::{params, Connection};

use super::generate_timeline_report;

fn setup_test_db(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER
        );
        CREATE TABLE session_summaries (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            request TEXT,
            completed TEXT,
            decisions TEXT,
            learned TEXT,
            next_steps TEXT,
            preferences TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0
        );
        CREATE TABLE memories (
            id INTEGER PRIMARY KEY,
            session_id TEXT,
            project TEXT NOT NULL,
            topic_key TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            files TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active'
        );
        CREATE TABLE ai_usage_events (
            id INTEGER PRIMARY KEY,
            created_at TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            project TEXT,
            operation TEXT NOT NULL,
            executor TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            estimated_cost_usd REAL NOT NULL
        );",
    )
    .unwrap();
}

#[test]
fn empty_project_produces_report() {
    let conn = Connection::open_in_memory().unwrap();
    setup_test_db(&conn);

    let report = generate_timeline_report(&conn, "tools/remem", false).unwrap();
    assert!(report.contains("# Journey Into tools/remem"));
    assert!(report.contains("Total observations: 0"));
}

#[test]
fn summary_report_excludes_timeline() {
    let conn = Connection::open_in_memory().unwrap();
    setup_test_db(&conn);
    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT INTO observations (memory_session_id, project, type, title, created_at_epoch, discovery_tokens) \
         VALUES ('s1', 'tools/remem', 'decision', 'Test observation', ?1, 100)",
        params![now],
    )
    .unwrap();

    let report = generate_timeline_report(&conn, "tools/remem", false).unwrap();
    assert!(report.contains("Total observations: 1"));
    assert!(!report.contains("## Timeline"));
    assert!(!report.contains("## Monthly Breakdown"));
}

#[test]
fn full_report_includes_timeline_and_monthly() {
    let conn = Connection::open_in_memory().unwrap();
    setup_test_db(&conn);
    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT INTO observations (memory_session_id, project, type, title, created_at_epoch, discovery_tokens) \
         VALUES ('s1', 'tools/remem', 'decision', 'FTS5 switch', ?1, 500)",
        params![now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO observations (memory_session_id, project, type, title, created_at_epoch, discovery_tokens) \
         VALUES ('s1', 'tools/remem', 'bugfix', 'Fix search', ?1, 300)",
        params![now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch) \
         VALUES ('s1', 'tools/remem', 'analyze search', '2026-03-19', ?1)",
        params![now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories (session_id, project, title, content, memory_type, created_at_epoch, updated_at_epoch) \
         VALUES ('s1', 'tools/remem', 'Test memory', 'content', 'decision', ?1, ?1)",
        params![now],
    )
    .unwrap();

    let report = generate_timeline_report(&conn, "tools/remem", true).unwrap();
    assert!(report.contains("## Timeline (recent first)"));
    assert!(report.contains("[decision] FTS5 switch"));
    assert!(report.contains("[bugfix] Fix search"));
    assert!(report.contains("## Monthly Breakdown"));
    assert!(report.contains("Total observations: 2"));
    assert!(report.contains("Total sessions: 1"));
    assert!(report.contains("Total memories: 1"));
}
