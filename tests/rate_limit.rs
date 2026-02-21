use anyhow::Result;
use rusqlite::{params, Connection};

use remem::{db, observe};

fn setup_observation_schema(conn: &Connection) -> Result<()> {
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
            discovery_tokens INTEGER DEFAULT 0,
            created_at TEXT,
            created_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER
        );

        CREATE VIRTUAL TABLE observations_fts USING fts5(
            title, subtitle, narrative, facts, concepts,
            content='observations',
            content_rowid='id'
        );

        CREATE TRIGGER observations_ai AFTER INSERT ON observations BEGIN
            INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
            VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
        END;

        CREATE TRIGGER observations_ad AFTER DELETE ON observations BEGIN
            INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
            VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
        END;

        CREATE TRIGGER observations_au AFTER UPDATE ON observations BEGIN
            INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
            VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
            INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
            VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
        END;",
    )?;
    Ok(())
}

#[test]
fn bash_skip_filter_stays_in_observe_module() {
    assert!(observe::should_skip_bash_command("git status"));
    assert!(observe::should_skip_bash_command("  ls -la  "));
    assert!(observe::should_skip_bash_command("cargo build --release"));
    assert!(!observe::should_skip_bash_command("git commit -m 'fix'"));
    assert!(!observe::should_skip_bash_command("cargo test"));
}

#[test]
fn project_key_is_stable_and_collision_resistant() {
    let a = db::project_from_cwd("/tmp/work/api");
    let b = db::project_from_cwd("/tmp/personal/api");
    assert_ne!(a, b);
    assert!(a.starts_with("work/api@"));
    let suffix = a.split('@').nth(1).unwrap_or_default();
    assert_eq!(suffix.len(), 12);
    assert_eq!(a, db::project_from_cwd("/tmp/work/api"));
}

#[test]
fn get_observations_by_ids_respects_project_filter() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;

    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, created_at, created_at_epoch, status)
         VALUES (1, 'm1', 'p1', 'feature', 'one', '2026-02-21T00:00:00Z', 1700000000, 'active')",
        [],
    )?;
    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, created_at, created_at_epoch, status)
         VALUES (2, 'm2', 'p2', 'feature', 'two', '2026-02-21T00:00:00Z', 1700000001, 'active')",
        [],
    )?;

    let all = db::get_observations_by_ids(&conn, &[1, 2], None)?;
    assert_eq!(all.len(), 2);

    let filtered = db::get_observations_by_ids(&conn, &[1, 2], Some("p1"))?;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, 1);
    Ok(())
}

#[test]
fn search_decay_prefers_newer_records_on_same_match() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let old = now - 60 * 86400;

    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, narrative, created_at, created_at_epoch, status)
         VALUES (?1, 'm1', 'p', 'feature', 'hello same', 'hello same', '2026-01-01T00:00:00Z', ?2, 'active')",
        params![1_i64, old],
    )?;
    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, narrative, created_at, created_at_epoch, status)
         VALUES (?1, 'm2', 'p', 'feature', 'hello same', 'hello same', '2026-02-21T00:00:00Z', ?2, 'active')",
        params![2_i64, now],
    )?;

    let results = db::search_observations_fts(&conn, "hello", Some("p"), None, 10, 0, true)?;
    assert!(results.len() >= 2);
    assert_eq!(results[0].id, 2);
    Ok(())
}
