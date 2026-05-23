use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::{
    add_preference, dedup_with_claude_md, query_global_preferences, query_project_preferences,
    remove_preference, render_preferences, render_preferences_with_limits,
    render_preferences_with_limits_detailed,
};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory()
        .unwrap_or_else(|err| panic!("Failed to open in-memory db: {err}"));
    crate::memory::types::tests_helper::setup_memory_schema(&conn);
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
fn test_project_preferences_exclude_global_overlay() -> Result<()> {
    let conn = setup_test_db();
    memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("local-pref"),
        "Preference: Local workflow",
        "Use the local project workflow",
        "preference",
        None,
    )?;
    memory::insert_memory_full(
        &conn,
        None,
        "other/proj",
        Some("global-pref"),
        "Preference: AtlasCloud markdown",
        "Verify AtlasCloud markdown rendering",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    let prefs = query_project_preferences(&conn, "test/proj", 20)?;
    assert_eq!(prefs.len(), 1);
    assert!(prefs[0].text.contains("local project workflow"));
    assert!(!prefs[0].text.contains("AtlasCloud"));
    Ok(())
}

#[test]
fn test_render_preferences_global_limit_zero_does_not_leak_global() -> Result<()> {
    let conn = setup_test_db();
    memory::insert_memory_full(
        &conn,
        None,
        "infra/aip",
        Some("atlas-pref"),
        "Preference: AtlasCloud markdown",
        "Verify AtlasCloud markdown rendering",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    let mut output = String::new();
    let rendered = render_preferences_with_limits(
        &mut output,
        &conn,
        "work/life/x",
        "/nonexistent",
        20,
        0,
        1500,
    )?;

    assert_eq!(rendered, 0);
    assert!(!output.contains("AtlasCloud"));
    Ok(())
}

#[test]
fn test_render_preferences_global_limit_explicitly_opted_in() -> Result<()> {
    let conn = setup_test_db();
    memory::insert_memory_full(
        &conn,
        None,
        "infra/aip",
        Some("atlas-pref"),
        "Preference: AtlasCloud markdown",
        "Verify AtlasCloud markdown rendering",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    let mut output = String::new();
    let rendered = render_preferences_with_limits(
        &mut output,
        &conn,
        "work/life/x",
        "/nonexistent",
        20,
        1,
        1500,
    )?;

    assert_eq!(rendered, 1);
    assert!(output.contains("AtlasCloud"));
    Ok(())
}

#[test]
fn test_render_preferences_reports_project_global_split() -> Result<()> {
    let conn = setup_test_db();
    memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("local-pref"),
        "Preference: Local workflow",
        "Use the local project workflow",
        "preference",
        None,
    )?;
    memory::insert_memory_full(
        &conn,
        None,
        "other/proj",
        Some("global-pref"),
        "Preference: Global reviews",
        "Review global release notes",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    let mut output = String::new();
    let summary = render_preferences_with_limits_detailed(
        &mut output,
        &conn,
        "test/proj",
        "/nonexistent",
        20,
        1,
        1500,
    )?;

    assert_eq!(summary.rendered, 2);
    assert_eq!(summary.project_rendered, 1);
    assert_eq!(summary.global_rendered, 1);
    assert!(output.contains("local project workflow"));
    assert!(output.contains("global release notes"));
    Ok(())
}

#[test]
fn test_global_preferences_require_explicit_global_scope() -> Result<()> {
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

    let global = query_global_preferences(&conn, 10)?;
    assert!(
        global.is_empty(),
        "Repeated project-scoped topic_key values must not become global preferences"
    );

    memory::insert_memory_full(
        &conn,
        None,
        "proj-a",
        Some("explicit-global"),
        "Preference: Explicit global",
        "Use explicit global preferences only",
        "preference",
        None,
        None,
        "global",
        None,
    )?;

    let global = query_global_preferences(&conn, 10)?;
    assert_eq!(global.len(), 1);
    assert!(global[0].text.contains("explicit global"));
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
