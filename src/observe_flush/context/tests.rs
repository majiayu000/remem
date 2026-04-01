use anyhow::Result;
use rusqlite::Connection;

use super::build_existing_context;

fn setup_existing_context_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE sdk_sessions (
            id INTEGER PRIMARY KEY,
            content_session_id TEXT UNIQUE NOT NULL,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            user_prompt TEXT,
            started_at TEXT,
            started_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            prompt_counter INTEGER DEFAULT 1
        );
        CREATE TABLE observations (
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
            last_accessed_epoch INTEGER,
            branch TEXT,
            commit_sha TEXT
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
            status TEXT NOT NULL DEFAULT 'active',
            branch TEXT,
            scope TEXT DEFAULT 'project'
        );",
    )?;
    Ok(())
}

#[test]
fn build_existing_context_includes_observations_and_memories() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_existing_context_schema(&conn)?;
    conn.execute(
        "INSERT INTO observations
         (memory_session_id, project, type, title, subtitle, created_at, created_at_epoch, status)
         VALUES ('mem-1', 'proj', 'feature', 'Observation title', 'Observation subtitle', '2026-01-01T00:00:00Z', 10, 'active')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (session_id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, scope)
         VALUES ('mem-2', 'proj', 'Memory title', 'Memory body first line
second line', 'decision', 20, 20, 'active', 'project')",
        [],
    )?;

    let xml = build_existing_context(&conn, "proj")?;
    assert!(xml.contains("source=\"observation\""));
    assert!(xml.contains("title=\"Observation title\""));
    assert!(xml.contains("Observation subtitle"));
    assert!(xml.contains("source=\"memory\""));
    assert!(xml.contains("title=\"Memory title\""));
    assert!(xml.contains("Memory body first line"));
    Ok(())
}
