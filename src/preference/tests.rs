use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::{
    add_preference, dedup_with_claude_md, query_global_preferences, remove_preference,
    render_preferences,
};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory()
        .unwrap_or_else(|err| panic!("Failed to open in-memory db: {err}"));
    conn.execute_batch(
        "CREATE TABLE memories (
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
        );
        CREATE VIRTUAL TABLE memories_fts USING fts5(
            title, content,
            content='memories',
            content_rowid='id',
            tokenize='trigram'
        );
        CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;
        CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;
        CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
        END;",
    )
    .unwrap_or_else(|err| panic!("Failed to create test schema: {err}"));
    conn
}

#[test]
fn test_render_preferences_empty() -> Result<()> {
    let conn = setup_test_db();
    let mut output = String::new();
    render_preferences(&mut output, &conn, "test/proj", ".")?;
    assert!(
        output.is_empty(),
        "Should not render section when no preferences"
    );
    Ok(())
}

#[test]
fn test_render_preferences_with_data() -> Result<()> {
    let conn = setup_test_db();
    memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("pref-1"),
        "Preference: Use Chinese comments",
        "Use Chinese comments in code",
        "preference",
        None,
    )?;

    let mut output = String::new();
    render_preferences(&mut output, &conn, "test/proj", "/nonexistent")?;
    assert!(output.contains("## Your Preferences"));
    assert!(output.contains("Use Chinese comments"));
    Ok(())
}

#[test]
fn test_global_preferences_threshold() -> Result<()> {
    let conn = setup_test_db();
    for project in &["proj-a", "proj-b", "proj-c"] {
        memory::insert_memory(
            &conn,
            None,
            project,
            Some("global-pref-1"),
            "Preference: Terse responses",
            "Give terse responses without summaries",
            "preference",
            None,
        )?;
    }
    memory::insert_memory(
        &conn,
        None,
        "proj-a",
        Some("local-pref"),
        "Preference: Use tabs",
        "Use tabs for indentation",
        "preference",
        None,
    )?;

    let global = query_global_preferences(&conn, 10)?;
    assert_eq!(
        global.len(),
        1,
        "Only preferences in 3+ projects should be returned"
    );
    assert!(global[0].text.contains("terse"));
    Ok(())
}

#[test]
fn test_dedup_with_claude_md() {
    let prefs = vec![
        Memory {
            id: 1,
            session_id: None,
            project: "test".into(),
            topic_key: Some("p1".into()),
            title: "Preference: use chinese comments".into(),
            text: "use chinese comments in code".into(),
            memory_type: "preference".into(),
            files: None,
            created_at_epoch: 0,
            updated_at_epoch: 0,
            status: "active".into(),
            branch: None,
            scope: "global".into(),
        },
        Memory {
            id: 2,
            session_id: None,
            project: "test".into(),
            topic_key: Some("p2".into()),
            title: "Preference: terse output".into(),
            text: "give terse output".into(),
            memory_type: "preference".into(),
            files: None,
            created_at_epoch: 0,
            updated_at_epoch: 0,
            status: "active".into(),
            branch: None,
            scope: "global".into(),
        },
    ];

    let indices = dedup_with_claude_md(&prefs, "/nonexistent");
    assert_eq!(indices.len(), 2, "All prefs should pass when no CLAUDE.md");
}

#[test]
fn test_add_and_remove_preference() -> Result<()> {
    let conn = setup_test_db();
    let id = add_preference(
        &conn,
        "test/proj",
        "Always use descriptive variable names",
        true,
    )?;
    assert!(id > 0);

    let prefs = memory::get_memories_by_type(&conn, "test/proj", "preference", 10)?;
    assert_eq!(prefs.len(), 1);

    let removed = remove_preference(&conn, id)?;
    assert!(removed);

    let prefs = memory::get_memories_by_type(&conn, "test/proj", "preference", 10)?;
    assert!(prefs.is_empty());
    Ok(())
}
