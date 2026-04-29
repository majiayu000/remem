use crate::memory::types::tests_helper::setup_memory_schema;
use crate::memory::Memory;
use crate::workstream::{WorkStream, WorkStreamStatus};
use rusqlite::{params, Connection};

use super::policy::{ContextLimits, ContextPolicy};
use super::query::{load_context_data, query_recent_summaries};
use super::render::enforce_total_char_limit;
use super::sections::{
    render_core_memory, render_memory_index, render_memory_index_with_limits,
    render_recent_sessions, render_workstreams, render_workstreams_with_limits,
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
fn render_memory_index_excludes_preferences() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "preference", "Preference title"),
        sample_memory(2, "decision", "Decision title"),
    ];

    render_memory_index(&mut output, &memories);

    assert!(output.contains("Decision title"));
    assert!(!output.contains("Preference title"));
    assert!(!output.contains("**Preferences**"));
}

#[test]
fn render_memory_index_respects_item_limit() {
    let mut output = String::new();
    let limits = ContextLimits {
        memory_index_limit: 2,
        ..ContextLimits::default()
    };
    let memories = vec![
        sample_memory(1, "decision", "Decision one"),
        sample_memory(2, "decision", "Decision two"),
        sample_memory(3, "decision", "Decision three"),
    ];

    render_memory_index_with_limits(&mut output, &memories, &limits);

    assert!(output.contains("Decision one"));
    assert!(output.contains("Decision two"));
    assert!(!output.contains("Decision three"));
}

#[test]
fn render_memory_index_truncates_first_item_to_char_limit() {
    let mut output = String::new();
    let limits = ContextLimits {
        memory_index_char_limit: 48,
        ..ContextLimits::default()
    };
    let long_title = "Decision title that is far too long for the index budget";
    let memories = vec![sample_memory(1, "decision", long_title)];

    let rendered = render_memory_index_with_limits(&mut output, &memories, &limits);
    let body = output.strip_prefix("## Index\n").unwrap().trim_end();

    assert_eq!(rendered, 1);
    assert!(body.chars().count() <= limits.memory_index_char_limit);
    assert!(output.contains("..."));
    assert!(!output.contains(long_title));
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
fn render_workstreams_respects_item_and_char_limits() {
    let mut output = String::new();
    let workstreams = vec![
        sample_workstream(1, "First stream", Some("ship the first fix")),
        sample_workstream(2, "Second stream", Some("ship the second fix")),
        sample_workstream(3, "Third stream", Some("ship the third fix")),
    ];

    render_workstreams_with_limits(&mut output, &workstreams, 2, 200);

    assert!(output.contains("#1 [active] First stream"));
    assert!(output.contains("#2 [active] Second stream"));
    assert!(!output.contains("#3 [active] Third stream"));
    assert!(output.chars().count() <= 200);
}

#[test]
fn render_workstreams_stops_at_char_limit() {
    let mut output = String::new();
    let workstreams = vec![
        sample_workstream(1, "First", Some("fix")),
        sample_workstream(2, "Second", Some("fix")),
    ];

    render_workstreams_with_limits(&mut output, &workstreams, 10, 48);

    assert!(output.contains("#1 [active] First"));
    assert!(!output.contains("#2 [active] Second"));
    assert!(output.chars().count() <= 48);
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
fn load_context_data_keeps_current_branch_memories_before_limit() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    for idx in 0..55 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("branchless-topic-{idx}")),
            "decision",
            &format!("Branchless decision {idx}"),
            "Recent branchless decision body",
            now - idx,
        );
    }
    insert_memory_with_branch(
        &conn,
        200,
        project,
        Some("current-branch-topic"),
        "bugfix",
        "Current branch operational fix",
        "Older but branch-specific fix body",
        now - 1_000,
        Some("fix/context-selection"),
    );

    let loaded = load_context_data(&conn, project, Some("fix/context-selection"));

    assert!(loaded.memories.len() > ContextLimits::default().memory_index_limit);
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Current branch operational fix"));
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

#[test]
fn load_context_data_excludes_preferences_from_main_memory_pool() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/computer";
    let now = chrono::Utc::now().timestamp();

    for idx in 0..60 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("preference-topic-{idx}")),
            "preference",
            &format!("Preference {idx}"),
            "User prefers evidence-backed coordination",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        100,
        project,
        Some("context-budget-decision"),
        "decision",
        "Use host-aware context compiler",
        "Split preferences from the main memory index",
        now - 100,
    );
    insert_memory(
        &conn,
        101,
        project,
        Some("context-budget-discovery"),
        "discovery",
        "Preference flood starves core memories",
        "The main index was dominated by preferences",
        now - 101,
    );

    let loaded = load_context_data(&conn, project, None);

    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.memory_type == "preference"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Use host-aware context compiler"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Preference flood starves core memories"));
}

#[test]
fn load_context_data_excludes_preferences_before_candidate_limit() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/computer";
    let now = chrono::Utc::now().timestamp();

    for idx in 0..130 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("preference-topic-{idx}")),
            "preference",
            &format!("Preference {idx}"),
            "User prefers evidence-backed coordination",
            now - idx,
        );
    }
    insert_memory(
        &conn,
        200,
        project,
        Some("context-budget-decision"),
        "decision",
        "Use host-aware context compiler",
        "Split preferences from the main memory index",
        now - 1_000,
    );
    insert_memory(
        &conn,
        201,
        project,
        Some("context-budget-bugfix"),
        "bugfix",
        "Keep core memories visible",
        "Filter preferences before applying the candidate cap",
        now - 1_001,
    );

    let loaded = load_context_data(&conn, project, None);

    assert!(loaded
        .memories
        .iter()
        .all(|memory| memory.memory_type != "preference"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Use host-aware context compiler"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Keep core memories visible"));
}

#[test]
fn enforce_total_char_limit_truncates_rendered_output() {
    let mut output = format!("{}{}", "# [/tmp/demo] context\n", "x".repeat(500));

    enforce_total_char_limit(&mut output, 120);

    assert!(output.chars().count() <= 120);
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn context_limits_env_override_and_legacy_alias_are_respected() {
    let mut vars = std::collections::HashMap::new();
    vars.insert("REMEM_CONTEXT_OBSERVATIONS", "7".to_string());
    vars.insert("REMEM_CONTEXT_CORE_ITEM_LIMIT", "3".to_string());
    vars.insert("REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT", "900".to_string());

    let limits = ContextLimits::from_env_reader(|key| vars.get(key).cloned());

    assert_eq!(limits.memory_index_limit, 7);
    assert_eq!(limits.core_item_limit, 3);
    assert_eq!(limits.preference_char_limit, 900);
}

#[test]
fn context_limits_new_memory_index_env_wins_over_legacy_alias() {
    let mut vars = std::collections::HashMap::new();
    vars.insert("REMEM_CONTEXT_OBSERVATIONS", "7".to_string());
    vars.insert("REMEM_CONTEXT_MEMORY_INDEX_LIMIT", "11".to_string());

    let limits = ContextLimits::from_env_reader(|key| vars.get(key).cloned());

    assert_eq!(limits.memory_index_limit, 11);
}

#[test]
fn load_context_data_keeps_core_candidates_when_index_limit_is_small() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        memory_index_limit: 1,
        core_item_limit: 4,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    for idx in 0..8 {
        insert_memory(
            &conn,
            idx + 1,
            project,
            Some(&format!("decision-topic-{idx}")),
            "decision",
            &format!("Decision {idx}"),
            "Decision body",
            now - idx,
        );
    }

    let loaded = super::query::load_context_data_with_policy(&conn, project, None, &policy);

    assert!(loaded.memories.len() > limits.memory_index_limit);
    assert!(loaded.memories.len() >= limits.core_item_limit);
}

#[test]
fn query_recent_summaries_filters_self_diagnostics_and_backfills() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE session_summaries (
            project TEXT,
            request TEXT,
            completed TEXT,
            created_at_epoch INTEGER
        );",
    )
    .unwrap();
    let project = "/tmp/vibeguard";

    insert_session_summary(
        &conn,
        project,
        "Debug remem context memory injection",
        Some("SessionStart memories loaded investigation"),
        300,
    );
    insert_session_summary(
        &conn,
        project,
        "Fix runtime hook",
        Some("Validated hook behavior"),
        299,
    );
    insert_session_summary(
        &conn,
        project,
        "Analyze SessionStart memories loaded",
        None,
        298,
    );
    insert_session_summary(
        &conn,
        project,
        "Review PR install paths",
        Some("Checked install scripts"),
        297,
    );
    insert_session_summary(
        &conn,
        project,
        "Memory injection follow-up",
        Some("remem context smoke test"),
        296,
    );
    insert_session_summary(
        &conn,
        project,
        "Repair guard source path",
        Some("Added source path evidence"),
        295,
    );

    let summaries = query_recent_summaries(&conn, project, 3).unwrap();

    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].request, "Fix runtime hook");
    assert_eq!(summaries[1].request, "Review PR install paths");
    assert_eq!(summaries[2].request, "Repair guard source path");
}

#[test]
fn query_recent_summaries_scans_past_self_diagnostic_burst() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";

    for idx in 0..30 {
        insert_session_summary(
            &conn,
            project,
            &format!("Debug remem context memory injection {idx}"),
            Some("SessionStart memories loaded investigation"),
            1_000 - idx,
        );
    }
    insert_session_summary(
        &conn,
        project,
        "Fix runtime hook",
        Some("Validated hook behavior"),
        100,
    );
    insert_session_summary(
        &conn,
        project,
        "Repair guard source path",
        Some("Added source path evidence"),
        99,
    );

    let summaries = query_recent_summaries(&conn, project, 2).unwrap();

    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].request, "Fix runtime hook");
    assert_eq!(summaries[1].request, "Repair guard source path");
}

#[test]
fn query_recent_summaries_suppresses_stale_design_prototype_noise() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_session_summary(
        &conn,
        project,
        "Build landing page and wireframe variants",
        Some("Starfield prototype shipped"),
        now - 8 * 86400,
    );
    insert_session_summary(
        &conn,
        project,
        "Generate VibeGuard wireframe prototype",
        Some("Landing page assets updated"),
        now - 9 * 86400,
    );
    insert_session_summary(
        &conn,
        project,
        "Fix runtime hook",
        Some("Validated hook behavior"),
        now - 10 * 86400,
    );

    let summaries = query_recent_summaries(&conn, project, 5).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].request, "Fix runtime hook");
}

#[test]
fn query_recent_summaries_keeps_stale_design_summary_as_last_resort() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_session_summary(
        &conn,
        project,
        "Build landing page and wireframe variants",
        Some("Starfield prototype shipped"),
        now - 8 * 86400,
    );

    let summaries = query_recent_summaries(&conn, project, 5).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].request,
        "Build landing page and wireframe variants"
    );
}

#[test]
fn query_recent_summaries_allows_normal_summary_after_low_signal_same_cluster() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_session_summary(
        &conn,
        project,
        "Review release work",
        Some("Starfield prototype shipped"),
        now - 8 * 86400,
    );
    insert_session_summary(
        &conn,
        project,
        "Review release work",
        Some("Validated current runtime hook behavior"),
        now - 9 * 86400,
    );

    let summaries = query_recent_summaries(&conn, project, 5).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].completed.as_deref(),
        Some("Validated current runtime hook behavior")
    );
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

fn sample_workstream(id: i64, title: &str, next_action: Option<&str>) -> WorkStream {
    WorkStream {
        id,
        project: "demo/project".to_string(),
        title: title.to_string(),
        description: None,
        status: WorkStreamStatus::Active,
        progress: None,
        next_action: next_action.map(str::to_string),
        blockers: None,
        created_at_epoch: 0,
        updated_at_epoch: id,
        completed_at_epoch: None,
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

fn insert_session_summary(
    conn: &Connection,
    project: &str,
    request: &str,
    completed: Option<&str>,
    created_at_epoch: i64,
) {
    conn.execute(
        "INSERT INTO session_summaries (project, request, completed, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4)",
        params![project, request, completed, created_at_epoch],
    )
    .unwrap();
}

fn create_session_summary_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE session_summaries (
            project TEXT,
            request TEXT,
            completed TEXT,
            created_at_epoch INTEGER
        );",
    )
    .unwrap();
}
