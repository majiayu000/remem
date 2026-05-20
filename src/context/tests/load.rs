use crate::memory::types::tests_helper::setup_memory_schema;
use rusqlite::Connection;

use super::super::policy::{ContextLimits, ContextPolicy};
use super::super::query::{load_context_data, load_context_data_with_policy};
use super::super::render::{enforce_total_char_limit, enforce_total_char_limit_preserving_footer};
use super::{insert_memory, insert_memory_with_branch, insert_memory_with_scope};

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
fn load_context_data_fetches_project_memories_without_global_candidate_flood() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    for idx in 0..10 {
        insert_memory_with_scope(
            &conn,
            idx + 1,
            "manual",
            Some(&format!("global-bugfix-{idx}")),
            "bugfix",
            &format!("Unrelated global bugfix {idx}"),
            "A global operational fix from another project",
            now - idx,
            "global",
        );
    }
    insert_memory(
        &conn,
        100,
        project,
        Some("local-bugfix"),
        "bugfix",
        "Project-local bugfix",
        "Keep remem SessionStart project context visible",
        now - 1_000,
    );
    insert_memory(
        &conn,
        101,
        project,
        Some("local-decision"),
        "decision",
        "Project-local decision",
        "Use project-only passive SessionStart memories",
        now - 1_001,
    );

    let loaded = load_context_data_with_policy(&conn, project, None, &policy);

    assert!(loaded
        .memories
        .iter()
        .all(|memory| memory.project == project && memory.scope == "project"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Project-local bugfix"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Project-local decision"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title.contains("Unrelated global bugfix")));
}

#[test]
fn load_context_data_filters_global_non_preferences_from_basename_search() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory_with_scope(
        &conn,
        1,
        "manual",
        Some("global-remem-bugfix"),
        "bugfix",
        "remem global duplicate-name bugfix",
        "A global memory mentioning remem should not enter passive startup context",
        now,
        "global",
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("local-remem-decision"),
        "decision",
        "remem local startup decision",
        "Project-local memories still participate in SessionStart",
        now - 1,
    );

    let loaded = load_context_data(&conn, project, None);

    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "remem local startup decision"));
    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title == "remem global duplicate-name bugfix"));
    assert!(loaded
        .memories
        .iter()
        .all(|memory| memory.scope == "project"));
}

#[test]
fn enforce_total_char_limit_truncates_rendered_output() {
    let mut output = format!("{}{}", "# [/tmp/demo] context\n", "x".repeat(500));

    enforce_total_char_limit(&mut output, 120);

    assert!(output.chars().count() <= 120);
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
}

#[test]
fn enforce_total_char_limit_preserves_footer_when_it_fits() {
    let footer = "22 context memories loaded. 2 core memories. 20 indexed memories. 5 preferences. 5 sessions.\n";
    let mut output = format!("{}{}{}", "# [/tmp/demo] context\n", "x".repeat(500), footer);

    enforce_total_char_limit_preserving_footer(&mut output, 180, footer);

    assert!(output.chars().count() <= 180);
    assert!(output.contains("REMEM_CONTEXT_TOTAL_CHAR_LIMIT"));
    assert!(output.ends_with(footer));
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

    let loaded = load_context_data_with_policy(&conn, project, None, &policy);

    assert!(loaded.memories.len() > limits.memory_index_limit);
    assert!(loaded.memories.len() >= limits.core_item_limit);
}
