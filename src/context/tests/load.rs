use crate::memory::lesson::{save_lesson, SaveLessonRequest};
use crate::memory::types::tests_helper::setup_memory_schema;
use rusqlite::Connection;

use super::super::policy::{ContextLimits, ContextPolicy};
use super::super::query::{load_context_data, load_context_data_with_policy};
use super::super::render::{enforce_total_char_limit, enforce_total_char_limit_preserving_footer};
use super::{insert_global_memory, insert_memory, insert_memory_with_branch};

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
fn load_context_data_loads_lessons_separately_from_main_memory_pool() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project,
            topic_key: Some("lesson-build-loop"),
            title: "Stop build-fix loops",
            content: "Lesson: after repeated build failures, stop and challenge the hypothesis.",
            confidence: 0.9,
            source_evidence: Some("build failed repeatedly"),
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: Some(now),
            stale_after_epoch: None,
        },
    )
    .unwrap();
    insert_memory(
        &conn,
        200,
        project,
        Some("context-budget-decision"),
        "decision",
        "Keep context bounded",
        "Separate lessons from the normal memory index",
        now - 1,
    );

    let loaded = load_context_data(&conn, project, None);

    assert_eq!(loaded.lessons.len(), 1);
    assert_eq!(loaded.lessons[0].memory.title, "Stop build-fix loops");
    assert!(loaded
        .memories
        .iter()
        .all(|memory| memory.memory_type != "lesson"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Keep context bounded"));
}

#[test]
fn load_context_data_filters_lessons_by_current_branch() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";

    for (title, branch) in [
        ("Main lesson", Some("main")),
        ("Feature lesson", Some("feature/context")),
    ] {
        save_lesson(
            &conn,
            &SaveLessonRequest {
                session_id: Some("s1"),
                project,
                topic_key: Some(&title.replace(' ', "-").to_lowercase()),
                title,
                content:
                    "Lesson: branch-specific lessons should follow the current context branch.",
                confidence: 0.9,
                source_evidence: None,
                files: None,
                branch,
                scope: "project",
                created_at_epoch: None,
                stale_after_epoch: None,
            },
        )
        .unwrap();
    }

    let loaded = load_context_data(&conn, project, Some("main"));
    let titles: Vec<_> = loaded
        .lessons
        .iter()
        .map(|lesson| lesson.memory.title.as_str())
        .collect();

    assert_eq!(titles, vec!["Main lesson"]);
}

#[test]
fn load_context_data_filters_lessons_before_candidate_limit_and_keeps_core_memory() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 1,
        lesson_limit: 10,
        memory_index_limit: 10,
        core_item_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    for idx in 0..20 {
        save_lesson(
            &conn,
            &SaveLessonRequest {
                session_id: Some("s1"),
                project,
                topic_key: Some(&format!("low-confidence-lesson-{idx}")),
                title: &format!("Low confidence lesson {idx}"),
                content: "Lesson: this should not enter context because confidence is too low.",
                confidence: 0.2,
                source_evidence: None,
                files: None,
                branch: None,
                scope: "project",
                created_at_epoch: Some(now + idx),
                stale_after_epoch: None,
            },
        )
        .unwrap();
    }
    insert_memory(
        &conn,
        200,
        project,
        Some("keep-core-decision"),
        "decision",
        "Keep core decision visible",
        "Low-confidence lessons must not crowd out core decisions.",
        now - 1_000,
    );

    let loaded = load_context_data_with_policy(&conn, project, None, &policy);

    assert!(loaded.lessons.is_empty());
    assert!(loaded
        .memories
        .iter()
        .all(|memory| memory.memory_type != "lesson"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Keep core decision visible"));
}

#[test]
fn load_context_data_excludes_global_non_preferences_from_main_memory_pool() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_global_memory(
        &conn,
        1,
        "manual",
        Some("mihomo-proxy-group-duplicate-name"),
        "bugfix",
        "Mihomo proxy group duplicate name",
        "This global bugfix should not appear in project SessionStart Core or Index.",
        now,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("context-project-only-memory"),
        "decision",
        "Keep context memory project scoped",
        "SessionStart should load project-local non-preference memories.",
        now - 1,
    );

    let loaded = load_context_data(&conn, project, None);

    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Mihomo proxy group duplicate name"));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Keep context memory project scoped"));
}

#[test]
fn load_context_data_excludes_global_non_preferences_from_basename_search() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();
    let limits = ContextLimits {
        candidate_fetch_limit: 1,
        memory_index_limit: 10,
        core_item_limit: 3,
        ..ContextLimits::default()
    };
    let policy = ContextPolicy::from_limits(limits);

    insert_memory(
        &conn,
        1,
        project,
        Some("older-local-memory"),
        "decision",
        "Older local remem decision",
        "A project-local decision should still appear through basename search.",
        now - 10,
    );
    insert_memory(
        &conn,
        2,
        project,
        Some("newer-local-memory"),
        "decision",
        "Newer local context decision",
        "This fills the tiny recent candidate limit.",
        now,
    );
    for idx in 0..25 {
        insert_global_memory(
            &conn,
            idx + 3,
            "manual",
            Some(&format!("global-remem-memory-{idx}")),
            "bugfix",
            &format!("Global remem bugfix {idx}"),
            "A global result matching the basename query should not enter context.",
            now + 1 + idx,
        );
    }

    let loaded = load_context_data_with_policy(&conn, project, None, &policy);

    assert!(!loaded
        .memories
        .iter()
        .any(|memory| memory.title.starts_with("Global remem bugfix")));
    assert!(loaded
        .memories
        .iter()
        .any(|memory| memory.title == "Older local remem decision"));
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
