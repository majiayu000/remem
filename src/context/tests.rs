use crate::memory::types::tests_helper::setup_memory_schema;
use crate::memory::Memory;
use crate::workstream::{WorkStream, WorkStreamStatus};
use rusqlite::{params, Connection};

use super::query::load_context_data;
use super::sections::{
    render_core_memory, render_memory_index, render_recent_sessions, render_workstreams,
};
use super::types::SessionSummaryBrief;

#[test]
fn render_recent_sessions_truncates_completed_line() {
    let mut output = String::new();
    let summaries = vec![SessionSummaryBrief {
        request: "Implement feature".to_string(),
        completed: Some(format!("{}\nignored", "x".repeat(130))),
        created_at_epoch: 1_710_000_000,
    }];

    render_recent_sessions(&mut output, &summaries);

    assert!(output.contains("Implement feature"));
    assert!(output.contains("=> "));
    assert!(output.contains("..."));
    assert!(!output.contains("ignored"));
}

#[test]
fn render_memory_index_prioritizes_known_types() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "custom", "Custom title"),
        sample_memory(2, "bugfix", "Fix title"),
        sample_memory(3, "decision", "Decision title"),
    ];

    render_memory_index(&mut output, &memories);

    let decision_pos = output.find("**Decisions**").unwrap();
    let bugfix_pos = output.find("**Bug Fixes**").unwrap();
    let custom_pos = output.find("**custom**").unwrap();
    assert!(decision_pos < bugfix_pos);
    assert!(bugfix_pos < custom_pos);
}

#[test]
fn render_workstreams_includes_next_action_when_present() {
    let mut output = String::new();
    let workstreams = vec![WorkStream {
        id: 7,
        project: "demo/project".to_string(),
        title: "Refactor context".to_string(),
        description: None,
        status: WorkStreamStatus::Active,
        progress: None,
        next_action: Some("split renderers".to_string()),
        blockers: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        completed_at_epoch: None,
    }];

    render_workstreams(&mut output, &workstreams);

    assert!(output.contains("#7 [active] Refactor context -> split renderers"));
}

#[test]
fn render_core_memory_prioritizes_higher_score_memories() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "discovery", "Lower score", now),
        sample_memory_with_epoch(2, "decision", "Higher score", now),
    ];

    render_core_memory(&mut output, &memories);

    let high_pos = output.find("**#2 Higher score**").unwrap();
    let low_pos = output.find("**#1 Lower score**").unwrap();
    assert!(high_pos < low_pos);
}

#[test]
fn render_core_memory_keeps_type_diversity_before_filling_same_type() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "decision", "Decision one", now),
        sample_memory_with_epoch(2, "decision", "Decision two", now),
        sample_memory_with_epoch(3, "decision", "Decision three", now),
        sample_memory_with_epoch(4, "discovery", "Operational discovery", now),
    ];

    render_core_memory(&mut output, &memories);

    let discovery_pos = output.find("**#4 Operational discovery**").unwrap();
    let third_decision_pos = output.find("**#3 Decision three**").unwrap();
    assert!(discovery_pos < third_decision_pos);
}

#[test]
fn render_core_memory_does_not_backfill_with_memory_self_diagnostics() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "decision", "Memory injection diagnosis", now),
        sample_memory_with_epoch(2, "discovery", "Runtime hook finding", now),
    ];

    render_core_memory(&mut output, &memories);

    let runtime_pos = output.find("**#2 Runtime hook finding**").unwrap();
    assert!(runtime_pos < output.len());
    assert!(!output.contains("Memory injection diagnosis"));
}

#[test]
fn render_core_memory_keeps_stale_decision_out_when_recent_context_is_available() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let stale_epoch = now - 8 * 86400;
    let memories = vec![
        sample_memory_with_epoch(1, "decision", "Recent decision one", now),
        sample_memory_with_epoch(3, "discovery", "Recent discovery one", now),
        sample_memory_with_epoch(4, "discovery", "Recent discovery two", now),
        sample_memory_with_epoch(5, "preference", "Recent preference one", now),
        sample_memory_with_epoch(6, "preference", "Recent preference two", now),
        sample_memory_with_epoch(7, "decision", "Stale landing page decision", stale_epoch),
    ];

    render_core_memory(&mut output, &memories);

    assert!(!output.contains("Stale landing page decision"));
}

#[test]
fn load_context_data_dedupes_generated_topic_keys_by_workstream_context() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    for idx in 0..6 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("decision-{:016x}", idx)),
            "decision",
            &format!("Landing page decision {idx}"),
            "[Context: Build VibeGuard landing page and wireframe variants]\nDecision body",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        99,
        project,
        Some("post-write-hook-worktree-hang"),
        "bugfix",
        "Fix post-write hook worktree hang",
        "Handle .git files when resolving worktree roots",
        now - 100,
    );

    let loaded = load_context_data(&conn, project, None);
    let landing_count = loaded
        .memories
        .iter()
        .filter(|memory| memory.title.contains("Landing page decision"))
        .count();

    assert_eq!(landing_count, 1);
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Fix post-write hook worktree hang"));
}

#[test]
fn load_context_data_clusters_contexts_by_pr_reference() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        1,
        project,
        Some("decision-1111111111111111"),
        "decision",
        "Skill install decision A",
        "[Context: Review VibeGuard PR #116 added agentsmd-audit and trajectory-review skills]\nBody",
        now,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("decision-2222222222222222"),
        "decision",
        "Skill install decision B",
        "[Context: Review VibeGuard PR 116 new skill installation contract]\nBody",
        now - 1,
    );

    let loaded = load_context_data(&conn, project, None);
    let pr_decision_count = loaded
        .memories
        .iter()
        .filter(|memory| memory.title.starts_with("Skill install decision"))
        .count();

    assert_eq!(pr_decision_count, 1);
}

#[test]
fn load_context_data_prefers_current_branch_within_dedup_cluster() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_memory_with_branch(
        &conn,
        1,
        project,
        Some("decision-1111111111111111"),
        "decision",
        "Main branch landing decision",
        "[Context: Build VibeGuard landing page and wireframe variants]\nMain body",
        now,
        Some("main"),
    );
    insert_memory_with_branch(
        &conn,
        2,
        project,
        Some("decision-2222222222222222"),
        "decision",
        "Feature branch landing decision",
        "[Context: Build VibeGuard landing page and wireframe variants]\nFeature body",
        now - 10,
        Some("fix/context-selection"),
    );

    let loaded = load_context_data(&conn, project, Some("fix/context-selection"));

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Feature branch landing decision"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Main branch landing decision"));
}

#[test]
fn load_context_data_limits_memory_self_diagnostics_before_index_rendering() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    for idx in 0..8 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("decision-{:016x}", idx)),
            "decision",
            &format!("Memory injection diagnosis {idx}"),
            "Debug remem context SessionStart memories loaded behavior",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        100,
        project,
        Some("guard-paths-source-path"),
        "bugfix",
        "Fix guard path source selection",
        "Use source path evidence in guard output",
        now - 20,
    );

    let loaded = load_context_data(&conn, project, None);
    let self_diagnostic_count = loaded
        .memories
        .iter()
        .filter(|memory| memory.title.contains("Memory injection diagnosis"))
        .count();

    assert_eq!(self_diagnostic_count, 2);
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Fix guard path source selection"));
}

fn sample_memory(id: i64, memory_type: &str, title: &str) -> Memory {
    sample_memory_with_epoch(id, memory_type, title, 1_710_000_000)
}

fn sample_memory_with_epoch(
    id: i64,
    memory_type: &str,
    title: &str,
    updated_at_epoch: i64,
) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "demo/project".to_string(),
        topic_key: None,
        title: title.to_string(),
        text: "Body".to_string(),
        memory_type: memory_type.to_string(),
        files: None,
        created_at_epoch: updated_at_epoch,
        updated_at_epoch,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

fn insert_memory(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
    updated_at_epoch: i64,
) {
    insert_memory_with_branch(
        conn,
        id,
        project,
        topic_key,
        memory_type,
        title,
        content,
        updated_at_epoch,
        None,
    );
}

fn insert_memory_with_branch(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    memory_type: &str,
    title: &str,
    content: &str,
    updated_at_epoch: i64,
    branch: Option<&str>,
) {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7, 'active', ?8, 'project')",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            updated_at_epoch,
            branch
        ],
    )
    .unwrap();
}
