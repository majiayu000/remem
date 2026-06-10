use rusqlite::{params, Connection};

use super::super::host::HostKind;
use super::super::policy::{ContextLimits, ContextPolicy};
use super::super::query::load_context_data;
use super::super::render::{build_context_debug_trace, ContextRenderStats, SectionRenderStats};
use super::super::types::{ContextRequest, LoadedContext};
use super::{insert_memory, insert_owned_memory, setup_context_schema};

#[test]
fn context_debug_reports_selected_ids_and_hidden_duplicate_groups() {
    let conn = setup_context_diagnostics_db();
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        1,
        project,
        Some("decision-1111111111111111"),
        "decision",
        "Old context diagnostics decision",
        "[Context: Build current view context diagnostics]\nOld body",
        now - 10,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("decision-2222222222222222"),
        "decision",
        "Current context diagnostics decision",
        "[Context: Build current view context diagnostics]\nCurrent body",
        now,
    );

    let loaded = load_context_data(&conn, project, None);
    let selected_titles = loaded
        .memories
        .iter()
        .map(|memory| memory.title.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        selected_titles,
        vec!["Current context diagnostics decision"]
    );
    assert_eq!(loaded.diagnostics.candidate_pool_total, 2);
    assert_eq!(loaded.diagnostics.selected_ids, vec![2]);
    assert_eq!(loaded.diagnostics.hidden_duplicate_groups.len(), 1);
    assert_eq!(loaded.diagnostics.hidden_duplicate_groups[0].chosen_id, 2);
    assert_eq!(
        loaded.diagnostics.hidden_duplicate_groups[0].hidden_ids,
        vec![1]
    );

    let debug = debug_trace(project, &loaded);
    assert!(debug.contains("selected_ids=[2]"));
    assert!(debug.contains("hidden duplicate group"));
    assert!(debug.contains("chosen=#2 hidden=[1]"));
}

#[test]
fn context_default_hides_stale_superseded_memory() {
    let conn = setup_context_diagnostics_db();
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        1,
        project,
        Some("current-toolbar-color"),
        "decision",
        "Toolbar color is emerald",
        "Toolbar color is emerald in the current UI.",
        now,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("stale-toolbar-color"),
        "decision",
        "Toolbar color was blue",
        "Toolbar color was blue in an old UI.",
        now - 1,
    );
    insert_memory(
        &conn,
        3,
        project,
        Some("superseded-toolbar-color"),
        "decision",
        "Toolbar color was purple",
        "Toolbar color was purple before emerald.",
        now - 2,
    );
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![2],
    )
    .unwrap();
    conn.execute(
        "UPDATE memories SET status = 'superseded' WHERE id = ?1",
        params![3],
    )
    .unwrap();

    let loaded = load_context_data(&conn, project, None);
    let titles = loaded
        .memories
        .iter()
        .map(|memory| memory.title.as_str())
        .collect::<Vec<_>>();

    assert_eq!(titles, vec!["Toolbar color is emerald"]);
    assert!(loaded
        .diagnostics
        .exclusions
        .iter()
        .any(|exclusion| exclusion.id == 2 && exclusion.reason == "stale"));
    assert!(loaded
        .diagnostics
        .exclusions
        .iter()
        .any(|exclusion| exclusion.id == 3 && exclusion.reason == "superseded"));
}

#[test]
fn context_default_hides_expired_operational_memory() {
    let conn = setup_context_diagnostics_db();
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        1,
        project,
        Some("stable-build-command"),
        "decision",
        "Run cargo check for context changes",
        "Use cargo check before submitting context changes.",
        now,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("expired-dev-server-port"),
        "discovery",
        "Dev server port 3000",
        "Dev server was temporarily running on port 3000.",
        now - 1,
    );
    conn.execute(
        "UPDATE memories SET expires_at_epoch = ?1 WHERE id = ?2",
        params![now - 1, 2],
    )
    .unwrap();

    let loaded = load_context_data(&conn, project, None);
    let titles = loaded
        .memories
        .iter()
        .map(|memory| memory.title.as_str())
        .collect::<Vec<_>>();

    assert_eq!(titles, vec!["Run cargo check for context changes"]);
    assert!(loaded
        .diagnostics
        .exclusions
        .iter()
        .any(|exclusion| exclusion.id == 2 && exclusion.reason == "expired"));
}

#[test]
fn context_debug_reports_stale_and_expired_exclusions() {
    let conn = setup_context_diagnostics_db();
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        1,
        project,
        Some("current-context-rule"),
        "decision",
        "Context debug stays read only",
        "Context diagnostics must not mutate memory rows.",
        now,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("stale-context-rule"),
        "decision",
        "Context debug applied cleanup",
        "Old behavior claimed context cleanup was allowed.",
        now - 1,
    );
    insert_memory(
        &conn,
        3,
        project,
        Some("expired-context-port"),
        "discovery",
        "Temporary context port",
        "A temporary server port expired before SessionStart.",
        now - 2,
    );
    conn.execute(
        "UPDATE memories SET status = 'stale' WHERE id = ?1",
        params![2],
    )
    .unwrap();
    conn.execute(
        "UPDATE memories SET expires_at_epoch = ?1 WHERE id = ?2",
        params![now - 1, 3],
    )
    .unwrap();

    let loaded = load_context_data(&conn, project, None);
    let debug = debug_trace(project, &loaded);

    assert!(debug.contains("excluded memory id=2 reason=stale"));
    assert!(debug.contains("excluded memory id=3 reason=expired"));
}

#[test]
fn context_state_key_current_view_selects_current_row_and_reports_ambiguity() {
    let conn = setup_context_diagnostics_db();
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        1,
        project,
        Some("old-verification-rule"),
        "decision",
        "Old verification rule",
        "Verification status and code changes can be mixed.",
        now - 10,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("current-verification-rule"),
        "decision",
        "Current verification rule",
        "Keep verification status separate from code changes.",
        now,
    );
    attach_state_key(
        &conn,
        10,
        project,
        "decision",
        "verification-status-separation",
        2,
        &[1, 2],
    );

    let loaded = load_context_data(&conn, project, None);
    let selected_ids = loaded
        .memories
        .iter()
        .map(|memory| memory.id)
        .collect::<Vec<_>>();

    assert_eq!(selected_ids, vec![2]);
    assert_eq!(loaded.diagnostics.selected_ids, vec![2]);
    assert_eq!(loaded.diagnostics.state_key_groups.len(), 1);
    let group = &loaded.diagnostics.state_key_groups[0];
    assert_eq!(group.current_id, Some(2));
    assert_eq!(group.active_ids, vec![1, 2]);
    assert_eq!(group.reason, "ambiguous_active_state_key_group");

    let debug = debug_trace(project, &loaded);
    assert!(debug.contains("state-key group"));
    assert!(debug.contains("current=#2 active=[1,2]"));
    assert!(debug.contains("reason=ambiguous_active_state_key_group"));
}

#[test]
fn context_debug_reports_preference_current_view_state_key_groups() {
    let conn = setup_context_diagnostics_db();
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_owned_memory(
        &conn,
        1,
        project,
        Some("pref-cn"),
        "preference",
        "Preference: 验证状态隔离",
        "验证状态必须和代码数据改动分开。",
        now - 10,
        "repo",
        project,
        Some(project),
        None,
    );
    insert_owned_memory(
        &conn,
        2,
        project,
        Some("pref-en"),
        "preference",
        "Preference: Verification status separation",
        "Keep verification status separate from code and data changes.",
        now,
        "repo",
        project,
        Some(project),
        None,
    );
    attach_state_key(
        &conn,
        11,
        project,
        "preference",
        "verification-status-separation",
        2,
        &[1, 2],
    );

    let mut preference_output = String::new();
    let details = crate::memory::preference::render_preferences_with_context_details(
        &mut preference_output,
        &conn,
        project,
        "/nonexistent",
        20,
        0,
        1500,
    )
    .unwrap();
    assert!(preference_output.contains("Keep verification status separate"));
    assert!(!preference_output.contains("验证状态必须"));
    assert_eq!(details.rendered_ids, vec![2]);

    let mut loaded = load_context_data(&conn, project, None);
    super::super::diagnostics::apply_preference_diagnostics(
        &conn,
        project,
        details.rendered_ids,
        &mut loaded.diagnostics,
    );
    let debug = debug_trace(project, &loaded);

    assert!(debug.contains("preference diagnostics selected_ids=[2]"));
    assert!(debug.contains("preference state-key group"));
    assert!(debug.contains("type=preference key=verification-status-separation"));
    assert!(debug.contains("current=#2 active=[1,2]"));
}

fn setup_context_diagnostics_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    conn
}

fn attach_state_key(
    conn: &Connection,
    state_key_id: i64,
    project: &str,
    memory_type: &str,
    state_key: &str,
    current_memory_id: i64,
    memory_ids: &[i64],
) {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'repo', ?2, ?3, ?4, 'active', ?5, 100, 100)",
        params![
            state_key_id,
            project,
            memory_type,
            state_key,
            current_memory_id
        ],
    )
    .unwrap();
    for memory_id in memory_ids {
        conn.execute(
            "UPDATE memories SET state_key_id = ?1 WHERE id = ?2",
            params![state_key_id, memory_id],
        )
        .unwrap();
    }
}

fn debug_trace(project: &str, loaded: &LoadedContext) -> String {
    let stats = ContextRenderStats {
        host: "codex-cli".to_string(),
        memories_loaded: loaded.memories.len(),
        core: SectionRenderStats {
            count: loaded.memories.len(),
            chars: 0,
        },
        owner_counts: loaded.owner_counts.clone(),
        ..ContextRenderStats::default()
    };
    let request = ContextRequest {
        cwd: project.to_string(),
        project: project.to_string(),
        session_id: Some("sess-diagnostics".to_string()),
        hook_source: None,
        current_branch: None,
        host: HostKind::CodexCli,
        use_colors: false,
    };
    build_context_debug_trace(
        &request,
        &ContextPolicy::from_limits(ContextLimits::default()),
        loaded,
        &stats,
    )
}
