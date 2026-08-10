use rusqlite::Connection;

use super::super::hybrid_context::query_hybrid_context_memories;
use super::{insert_memory, setup_context_schema};

#[test]
fn hybrid_context_resolves_historical_project_aliases() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_context_schema(&conn);
    conn.execute_batch(
        "CREATE TABLE projects (id INTEGER PRIMARY KEY, project_path TEXT NOT NULL);
         CREATE TABLE project_identity_aliases (
             alias_path TEXT PRIMARY KEY,
             canonical_project_id INTEGER NOT NULL,
             status TEXT NOT NULL
         );
         INSERT INTO projects(id, project_path) VALUES(1, '/new/repo');
         INSERT INTO project_identity_aliases(alias_path, canonical_project_id, status)
         VALUES('/old/repo', 1, 'active');",
    )?;
    insert_memory(
        &conn,
        1,
        "/old/repo",
        Some("alias-contract"),
        "decision",
        "Historical alias memory",
        "The old checkout belongs to the canonical project.",
        chrono::Utc::now().timestamp(),
    );

    let memories = query_hybrid_context_memories(
        &conn,
        "/new/repo",
        "old checkout canonical project",
        None,
        &[],
        5,
        false,
    )?;

    assert!(memories
        .iter()
        .any(|memory| memory.title == "Historical alias memory"));
    Ok(())
}
