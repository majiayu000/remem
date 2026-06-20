use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::suppression::{create_suppression, parse_target, SuppressRequest};
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
fn test_project_preferences_exclude_tool_owned_rows_from_same_project() -> Result<()> {
    let conn = setup_test_db();
    let repo_pref = memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("local-pref"),
        "Preference: Local workflow",
        "Use the local project workflow",
        "preference",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET source_project = project, target_project = project,
             owner_scope = 'repo', owner_key = project
         WHERE id = ?1",
        [repo_pref],
    )?;
    let tool_pref = memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("codex-pref"),
        "Preference: Codex approvals",
        "Use Codex workspace-write approval mode",
        "preference",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET source_project = project, target_project = NULL,
             owner_scope = 'tool', owner_key = 'codex-cli'
         WHERE id = ?1",
        [tool_pref],
    )?;

    let prefs = query_project_preferences(&conn, "test/proj", 20)?;
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].text, "Use the local project workflow");
    Ok(())
}

#[test]
fn project_preferences_exclude_policy_suppressed_rows() -> Result<()> {
    let conn = setup_test_db();
    memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("visible-pref"),
        "Preference: Visible",
        "Use visible preference",
        "preference",
        None,
    )?;
    let hidden = memory::insert_memory(
        &conn,
        None,
        "test/proj",
        Some("hidden-pref"),
        "Preference: Hidden",
        "Do not render hidden preference",
        "preference",
        None,
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target(&format!("memory:{hidden}"))?,
            reason: Some("too noisy"),
            actor: Some("test"),
        },
    )?;

    let prefs = query_project_preferences(&conn, "test/proj", 20)?;

    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].text, "Use visible preference");
    Ok(())
}

#[test]
fn test_user_owner_preferences_are_global_core_preferences() -> Result<()> {
    let conn = setup_test_db();
    let id = memory::insert_memory_full(
        &conn,
        None,
        "other/proj",
        Some("user-style"),
        "Preference: User style",
        "Keep answers concise",
        "preference",
        None,
        None,
        "global",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'user', owner_key = 'user:default'
         WHERE id = ?1",
        [id],
    )?;

    let prefs = query_global_preferences(&conn, 10)?;
    assert_eq!(prefs.len(), 1);
    assert_eq!(prefs[0].text, "Keep answers concise");
    Ok(())
}

#[test]
fn test_global_preferences_keep_same_topic_across_owner_groups() -> Result<()> {
    let conn = setup_test_db();
    insert_preference_row(
        &conn,
        1,
        "legacy/proj",
        Some("shared-style"),
        "Preference: Shared style",
        "Legacy global style preference",
        "global",
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = NULL, owner_key = NULL
         WHERE id = ?1",
        [1],
    )?;
    insert_preference_row(
        &conn,
        2,
        "user/proj",
        Some("shared-style"),
        "Preference: Shared style",
        "User-owned style preference",
        "global",
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'user', owner_key = 'user:default'
         WHERE id = ?1",
        [2],
    )?;

    let global = query_global_preferences(&conn, 10)?;
    let ids = global.iter().map(|memory| memory.id).collect::<Vec<_>>();
    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert_eq!(global.len(), 2);
    Ok(())
}

#[test]
fn add_preference_reuses_semantic_near_duplicate() -> Result<()> {
    let conn = setup_test_db();

    let first_id = add_preference(
        &conn,
        "test/proj",
        "Prefer small reversible changes and include verification output for every fix.",
        false,
    )?;
    let second_id = add_preference(
        &conn,
        "test/proj",
        "Prefer small reversible code changes with verification output for each fix.",
        false,
    )?;

    assert_eq!(second_id, first_id);
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE project = 'test/proj'
           AND memory_type = 'preference'
           AND status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 1);
    Ok(())
}

#[test]
fn semantic_preference_dedup_keeps_project_scope_isolated() -> Result<()> {
    let conn = setup_test_db();

    let first_id = add_preference(
        &conn,
        "first/proj",
        "Prefer small reversible changes and include verification output for every fix.",
        false,
    )?;
    let second_id = add_preference(
        &conn,
        "second/proj",
        "Prefer small reversible code changes with verification output for each fix.",
        false,
    )?;

    assert_ne!(second_id, first_id);
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE memory_type = 'preference'
           AND status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 2);
    Ok(())
}

#[test]
fn semantic_preference_dedup_keeps_opposite_preferences_separate() -> Result<()> {
    let conn = setup_test_db();

    let first_id = add_preference(
        &conn,
        "test/proj",
        "Never force push branches; require explicit approval before rewriting history.",
        false,
    )?;
    let second_id = add_preference(
        &conn,
        "test/proj",
        "Always force push branches when they are behind; do not ask for approval.",
        false,
    )?;

    assert_ne!(second_id, first_id);
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE project = 'test/proj'
           AND memory_type = 'preference'
           AND status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 2);
    Ok(())
}

#[test]
fn test_preferences_use_state_key_current_view_per_owner() -> Result<()> {
    let conn = setup_test_db();
    insert_preference_row(
        &conn,
        1,
        "test/proj",
        Some("pref-cn"),
        "Preference: 验证状态隔离",
        "验证状态必须和代码数据改动分开。",
        "project",
    )?;
    insert_preference_row(
        &conn,
        2,
        "test/proj",
        Some("pref-en"),
        "Preference: Verification status separation",
        "Keep verification status separate from code and data changes.",
        "project",
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'repo', owner_key = 'test/proj', target_project = 'test/proj'
         WHERE id IN (1, 2)",
        [],
    )?;
    attach_preference_state_key(&conn, 10, "repo", "test/proj", 2, &[1, 2])?;

    insert_preference_row(
        &conn,
        3,
        "other/proj",
        Some("pref-user"),
        "Preference: Verification status separation",
        "User-level verification status preference remains separate.",
        "global",
    )?;
    conn.execute(
        "UPDATE memories
         SET owner_scope = 'user', owner_key = 'user:default'
         WHERE id = 3",
        [],
    )?;
    attach_preference_state_key(&conn, 11, "user", "user:default", 3, &[3])?;

    let project = query_project_preferences(&conn, "test/proj", 20)?;
    assert_eq!(
        project.iter().map(|memory| memory.id).collect::<Vec<_>>(),
        vec![2]
    );
    assert!(project[0].text.contains("separate from code"));

    let global = query_global_preferences(&conn, 10)?;
    assert_eq!(
        global.iter().map(|memory| memory.id).collect::<Vec<_>>(),
        vec![3]
    );
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

fn insert_preference_row(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    scope: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'preference', NULL, ?6, ?6, 'active', NULL, ?7)",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            1_710_000_000 + id,
            scope
        ],
    )?;
    Ok(())
}

fn attach_preference_state_key(
    conn: &Connection,
    state_key_id: i64,
    owner_scope: &str,
    owner_key: &str,
    current_memory_id: i64,
    memory_ids: &[i64],
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, 'preference', 'verification-status-separation',
                 'active', ?4, 100, 100)",
        params![state_key_id, owner_scope, owner_key, current_memory_id],
    )?;
    for memory_id in memory_ids {
        conn.execute(
            "UPDATE memories SET state_key_id = ?1 WHERE id = ?2",
            params![state_key_id, memory_id],
        )?;
    }
    Ok(())
}
