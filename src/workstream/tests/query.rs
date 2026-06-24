use rusqlite::{params, Connection};

use super::support::setup_workstream_schema;
use crate::workstream::{
    find_matching_workstream, query_active_workstreams, query_workstreams, upsert_workstream,
    upsert_workstream_with_match, ParsedWorkStream, WorkStreamStatus,
};

#[test]
fn test_upsert_creates_new() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: Some("Implement WorkStream".to_string()),
        progress: Some("Started design".to_string()),
        next_action: Some("Write code".to_string()),
        blockers: None,
        is_completed: false,
    };
    let id = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();
    assert!(id > 0);

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].title, "Implement WorkStream");
    assert_eq!(workstreams[0].status, WorkStreamStatus::Active);
}

#[test]
fn test_upsert_updates_existing() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed1 = ParsedWorkStream {
        title: Some("Feature X".to_string()),
        progress: Some("Step 1 done".to_string()),
        next_action: Some("Step 2".to_string()),
        blockers: None,
        is_completed: false,
    };
    let id1 = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

    let parsed2 = ParsedWorkStream {
        title: Some("Feature X".to_string()),
        progress: Some("Step 2 done".to_string()),
        next_action: Some("Step 3".to_string()),
        blockers: None,
        is_completed: false,
    };
    let id2 = upsert_workstream(&conn, "test/proj", "mem-def", &parsed2).unwrap();
    assert_eq!(id1, id2);

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].progress.as_deref(), Some("Step 2 done"));
}

#[test]
fn test_fuzzy_match() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: Some("WorkStream 层实现".to_string()),
        progress: Some("设计完成".to_string()),
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();

    let found = find_matching_workstream(&conn, "test/proj", "WorkStream").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().title, "WorkStream 层实现");
}

#[test]
fn test_no_match_creates_new() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed1 = ParsedWorkStream {
        title: Some("Feature A".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

    let parsed2 = ParsedWorkStream {
        title: Some("Feature B".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-def", &parsed2).unwrap();

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 2);
}

#[test]
fn same_session_rename_chain_keeps_one_canonical_workstream() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    for title in [
        "agent-workflow Skill 生命周期工作流",
        "flowguard Skill 生命周期工作流",
        "flowguard / run-guard Skill 生命周期工作流",
    ] {
        let parsed = ParsedWorkStream {
            title: Some(title.to_string()),
            progress: Some(format!("progress for {title}")),
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        upsert_workstream(&conn, "test/proj", "mem-spellbook", &parsed).unwrap();
    }

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(
        workstreams[0].title,
        "flowguard / run-guard Skill 生命周期工作流"
    );

    let alias_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workstream_aliases WHERE workstream_id = ?1",
            params![workstreams[0].id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(alias_count, 3);

    let session_links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workstream_sessions WHERE workstream_id = ?1",
            params![workstreams[0].id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(session_links, 1);
}

#[test]
fn same_session_unrelated_title_does_not_update_prior_workstream() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    for title in [
        "agent-workflow Skill 生命周期工作流",
        "release notes cleanup",
    ] {
        let parsed = ParsedWorkStream {
            title: Some(title.to_string()),
            progress: None,
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        upsert_workstream(&conn, "test/proj", "mem-shared", &parsed).unwrap();
    }

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 2);
}

#[test]
fn same_session_titles_sharing_one_domain_token_do_not_merge() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    for title in ["billing import cleanup", "billing dashboard rollout"] {
        let parsed = ParsedWorkStream {
            title: Some(title.to_string()),
            progress: None,
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        upsert_workstream(&conn, "test/proj", "mem-shared", &parsed).unwrap();
    }

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 2);
}

#[test]
fn alias_exact_match_updates_canonical_workstream_from_later_session() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let original = ParsedWorkStream {
        title: Some("agent-workflow Skill 生命周期工作流".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let canonical_id = upsert_workstream(&conn, "test/proj", "mem-first", &original).unwrap();
    let renamed = ParsedWorkStream {
        title: Some("flowguard Skill 生命周期工作流".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    upsert_workstream(&conn, "test/proj", "mem-first", &renamed).unwrap();

    let later = ParsedWorkStream {
        title: Some("agent-workflow Skill 生命周期工作流".to_string()),
        progress: Some("later session reused old title".to_string()),
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let result = upsert_workstream_with_match(&conn, "test/proj", "mem-later", &later).unwrap();

    assert_eq!(result.id, canonical_id);
    assert_eq!(result.match_reason, "alias_exact");
    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(
        workstreams[0].progress.as_deref(),
        Some("later session reused old title")
    );
}

#[test]
fn merged_duplicate_rows_are_hidden_from_active_queries_and_matchers() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let canonical = ParsedWorkStream {
        title: Some("Canonical Task".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let canonical_id = upsert_workstream(&conn, "test/proj", "mem-a", &canonical).unwrap();
    let duplicate = ParsedWorkStream {
        title: Some("Duplicate Task".to_string()),
        progress: None,
        next_action: None,
        blockers: None,
        is_completed: false,
    };
    let duplicate_id = upsert_workstream(&conn, "test/proj", "mem-b", &duplicate).unwrap();
    conn.execute(
        "UPDATE workstreams SET merged_into_workstream_id = ?1 WHERE id = ?2",
        params![canonical_id, duplicate_id],
    )
    .unwrap();

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0].id, canonical_id);

    let found = find_matching_workstream(&conn, "test/proj", "Duplicate Task").unwrap();
    assert!(found.is_none());
}

#[test]
fn test_only_matches_active_or_paused() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES ('test/proj', 'Old Task', 'completed', ?1, ?1)",
        params![now],
    )
    .unwrap();

    let found = find_matching_workstream(&conn, "test/proj", "Old Task").unwrap();
    assert!(found.is_none());
}

#[test]
fn test_completed_status() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);

    let parsed = ParsedWorkStream {
        title: Some("Done Task".to_string()),
        progress: Some("All done".to_string()),
        next_action: None,
        blockers: None,
        is_completed: true,
    };
    upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();

    let active = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(active.len(), 0);

    let completed = query_workstreams(&conn, "test/proj", Some("completed")).unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].status, WorkStreamStatus::Completed);
    assert!(completed[0].completed_at_epoch.is_some());
}

#[test]
fn test_query_active_workstreams_excludes_paused_repo_rows() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);
    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
         VALUES
         ('test/proj', 'Active Task', 'active', ?1, ?1),
         ('test/proj', 'Paused Task', 'paused', ?1, ?1)",
        params![now],
    )
    .unwrap();

    let active = query_active_workstreams(&conn, "test/proj").unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].title, "Active Task");
}

#[test]
fn test_query_active_workstreams_excludes_tool_domain_owned_rows() {
    let conn = Connection::open_in_memory().unwrap();
    setup_workstream_schema(&conn);
    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key)
         VALUES
         ('test/proj', 'Repo Task', 'active', ?1, ?1, 'test/proj', 'test/proj', 'repo', 'test/proj'),
         ('test/proj', 'Codex Task', 'active', ?1, ?1, 'test/proj', NULL, 'tool', 'codex-cli'),
         ('test/proj', 'Grok Task', 'paused', ?1, ?1, 'test/proj', NULL, 'domain', 'grok-api')",
        params![now],
    )
    .unwrap();

    let workstreams = query_active_workstreams(&conn, "test/proj").unwrap();
    let titles = workstreams
        .iter()
        .map(|workstream| workstream.title.as_str())
        .collect::<Vec<_>>();

    assert_eq!(titles, vec!["Repo Task"]);
}
