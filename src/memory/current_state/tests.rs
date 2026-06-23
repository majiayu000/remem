use anyhow::Result;
use rusqlite::params;

use super::{current_state, CurrentStateRequest};
use support::*;

mod review_regressions;
mod support;

#[test]
fn returns_current_answer_when_state_key_has_single_active_current() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert_eq!(
        result
            .current
            .as_ref()
            .map(|memory| memory.staleness.source_anchor.as_str()),
        Some("untracked")
    );
    assert!(result.conflicts.is_empty());
    Ok(())
}

#[test]
fn current_state_scrubs_policy_suppressed_current_memory_id() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target("memory:2")?,
            reason: Some("do not show"),
            actor: Some("test"),
        },
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "no_current");
    assert!(result.current.is_none());
    assert_eq!(
        result
            .state
            .as_ref()
            .and_then(|state| state.current_memory_id),
        None
    );
    let json = serde_json::to_value(&result)?;
    assert!(!json.to_string().contains("current_memory_id"));
    Ok(())
}

#[test]
fn ambiguous_current_state_scrubs_only_policy_suppressed_match_ids() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key_for(&conn, 10, "/repo", None)?;
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (11, 'user', 'user:default', 'decision', 'deploy-target',
                 'deploy target', 'active', NULL, 1, 10)",
        [],
    )?;
    insert_current_state_memory(
        &conn,
        2,
        "Repo deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    insert_current_state_memory(
        &conn,
        3,
        "User deploy target",
        "Use staging.",
        "active",
        11,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE memory_state_keys
         SET current_memory_id = CASE id WHEN 10 THEN 2 WHEN 11 THEN 3 END
         WHERE id IN (10, 11)",
        [],
    )?;
    crate::memory::suppression::create_suppression(
        &conn,
        &crate::memory::suppression::SuppressRequest {
            target: crate::memory::suppression::parse_target("memory:2")?,
            reason: Some("do not show"),
            actor: Some("test"),
        },
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "ambiguous");
    assert_eq!(result.matches.len(), 2);
    let repo_match = result
        .matches
        .iter()
        .find(|item| item.owner_scope == "repo")
        .expect("repo match should be present");
    let user_match = result
        .matches
        .iter()
        .find(|item| item.owner_scope == "user")
        .expect("user match should be present");
    assert_eq!(repo_match.current_memory_id, None);
    assert_eq!(user_match.current_memory_id, Some(3));
    let json = serde_json::to_value(&result)?;
    assert!(!json.to_string().contains("\"current_memory_id\":2"));
    Ok(())
}

#[test]
fn shows_superseded_history_and_edge_evidence_for_current_answer() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        1,
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('supersedes', 1, 2, 10, '[7,8]', 'new deploy decision', 300)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "current");
    assert_eq!(result.history.len(), 1);
    assert_eq!(result.history[0].id, 1);
    assert_eq!(result.history[0].relation.as_deref(), Some("supersedes"));
    assert_eq!(result.history[0].evidence_event_ids, vec![7, 8]);
    assert_eq!(result.history[0].staleness.source_anchor, "untracked");
    assert_eq!(result.why[0].reason.as_deref(), Some("new deploy decision"));
    Ok(())
}

#[test]
fn history_preserves_edge_for_legacy_superseded_row_without_state_key() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        1,
        "Legacy deploy target",
        "Use staging.",
        "stale",
        10,
        Some(100),
        Some(200),
    )?;
    conn.execute("UPDATE memories SET state_key_id = NULL WHERE id = 1", [])?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('supersedes', 1, 2, 10, '[9]', 'legacy replacement', 300)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "current");
    assert_eq!(result.history.len(), 1);
    assert_eq!(result.history[0].id, 1);
    assert_eq!(result.history[0].relation.as_deref(), Some("supersedes"));
    assert_eq!(result.history[0].evidence_event_ids, vec![9]);
    assert_eq!(
        result.history[0].reason.as_deref(),
        Some("legacy replacement")
    );
    Ok(())
}

#[test]
fn unresolved_active_conflict_does_not_silently_choose_current_pointer() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    insert_current_state_memory(
        &conn,
        3,
        "Deploy target conflict",
        "Use staging.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('conflicts', 3, 2, 10, 'operator conflict', 300)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "unresolved_conflict");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].id, 3);
    assert_eq!(result.conflicts[0].staleness.source_anchor, "untracked");
    Ok(())
}

#[test]
fn staleness_labels_cover_tracked_current_and_verify_before_trust_conflict() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    insert_current_state_memory(
        &conn,
        3,
        "Deploy target conflict",
        "Use staging.",
        "active",
        10,
        None,
        None,
    )?;
    set_memory_source(&conn, 2, "session-current", &["src/current.rs"])?;
    set_memory_source(&conn, 3, "session-conflict", &["src/conflict.rs"])?;
    link_current_state_commit(
        &conn,
        100,
        "source-current",
        100,
        &["src/current.rs"],
        "session-current",
    )?;
    link_current_state_commit(
        &conn,
        101,
        "source-conflict",
        100,
        &["src/conflict.rs"],
        "session-conflict",
    )?;
    insert_current_state_commit(&conn, 102, "later-conflict", 200, &["src/conflict.rs"])?;
    set_current_memory(&conn, 2)?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.status, "unresolved_conflict");
    assert_eq!(
        result
            .current
            .as_ref()
            .map(|memory| memory.staleness.source_anchor.as_str()),
        Some("tracked")
    );
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(
        result.conflicts[0].staleness.source_anchor,
        "verify-before-trust"
    );
    Ok(())
}

#[test]
fn source_anchor_errors_are_reported_per_current_state_ref() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_memory_files_raw(&conn, 2, "[not-json")?;
    set_current_memory(&conn, 2)?;

    let result = current_state(&conn, &request())?;
    let staleness = &result.current.as_ref().expect("current memory").staleness;

    assert_eq!(staleness.source_anchor, "error");
    assert!(staleness
        .error
        .as_deref()
        .is_some_and(|error| { error.contains("source-anchor staleness") }));
    Ok(())
}

#[test]
fn as_of_time_uses_memory_validity_window() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        1,
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert_eq!(
        result.current.as_ref().map(|memory| memory.status.as_str()),
        Some("stale")
    );
    Ok(())
}

#[test]
fn as_of_conflict_refs_include_staleness_labels() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Old deploy target",
        "Use staging.",
        "active",
        10,
        100,
        None,
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        120,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "unresolved_conflict");
    assert!(result.current.is_none());
    assert_eq!(result.conflicts.len(), 2);
    assert!(result
        .conflicts
        .iter()
        .all(|memory| memory.staleness.source_anchor == "untracked"));
    Ok(())
}

#[test]
fn omitted_project_defaults_to_current_repo_scope() -> Result<()> {
    let conn = current_state_test_conn()?;
    let cwd = std::env::current_dir()?;
    let current_project = crate::db::project_from_cwd(cwd.to_string_lossy().as_ref());
    insert_state_key_for(&conn, 10, &current_project, None)?;
    insert_state_key_for(&conn, 11, "/other/repo", None)?;
    insert_current_state_memory_at(
        &conn,
        2,
        &current_project,
        "Current repo deploy target",
        "Use production.",
        "active",
        10,
        100,
        None,
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        3,
        "/other/repo",
        "Other repo deploy target",
        "Use staging.",
        "active",
        11,
        100,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = 2 WHERE id = 10",
        [],
    )?;
    conn.execute(
        "UPDATE memory_state_keys SET current_memory_id = 3 WHERE id = 11",
        [],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            project: None,
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert!(result.matches.is_empty());
    Ok(())
}

#[test]
fn as_of_time_excludes_memories_created_after_cutoff_when_valid_from_is_null() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        100,
        None,
        Some(200),
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Future deploy target",
        "Use production.",
        "active",
        10,
        300,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    Ok(())
}

#[test]
fn as_of_time_includes_archived_memories_inside_validity_window() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Archived deploy target",
        "Use staging.",
        "archived",
        10,
        100,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        200,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "current");
    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert_eq!(
        result.current.as_ref().map(|memory| memory.status.as_str()),
        Some("archived")
    );
    Ok(())
}

#[test]
fn as_of_time_excludes_archived_memories_without_valid_to_epoch() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Archived deploy target",
        "Use staging.",
        "archived",
        10,
        100,
        Some(100),
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Future deploy target",
        "Use production.",
        "active",
        10,
        300,
        Some(300),
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.status, "no_current");
    assert!(result.current.is_none());
    Ok(())
}

#[test]
fn current_answer_facts_exclude_expired_and_future_active_facts() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;
    let now = chrono::Utc::now().timestamp();
    for (id, object, valid_from, valid_to) in [
        (1_i64, "production", now - 100, Some(now + 100)),
        (2_i64, "expired", now - 200, Some(now - 100)),
        (3_i64, "future", now + 100, None),
    ] {
        conn.execute(
            "INSERT INTO memory_facts
             (id, project, subject, predicate, object, valid_from_epoch,
              valid_to_epoch, learned_at_epoch, source_memory_id, source_event_ids,
              confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, '/repo', 'deploy-target', 'affects_project', ?2, ?3,
                     ?4, ?3, 2, '[]', 0.9, 'active', ?3, ?3)",
            params![id, object, valid_from, valid_to],
        )?;
    }

    let result = current_state(&conn, &request())?;

    assert_eq!(
        result
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["production"]
    );
    Ok(())
}

#[test]
fn as_of_time_excludes_expired_memory_before_cleanup_sets_valid_to() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_with_expiry_at(
        &conn,
        2,
        "/repo",
        "Temporary deploy target",
        "Use production until expiry.",
        "active",
        10,
        100,
        Some(200),
        Some(100),
        None,
    )?;
    set_current_memory(&conn, 2)?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(250),
            ..request()
        },
    )?;

    assert_eq!(result.status, "no_current");
    assert!(result.current.is_none());
    Ok(())
}

#[test]
fn why_includes_derived_from_provenance_for_current_answer() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        None,
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (20, NULL, 'project', 'decision', 'deploy-target', 'Use production.',
                 '[11,12]', 0.9, 'low', 'approved', 90, 90)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_operation_log
         (id, operation, planner_version, actor, source, owner_scope, owner_key,
          memory_type, state_key, source_candidate_id, result_memory_id, created_at_epoch)
         VALUES (30, 'add', 'test', 'test', 'test', 'repo', '/repo',
                 'decision', 'deploy-target', 20, 2, 95)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, source_candidate_id,
          evidence_event_ids, source_operation_id, reason, created_at_epoch)
         VALUES ('derived_from', NULL, 2, NULL, 20, '[11,12]', 30,
                 'candidate promoted', 100)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(result.why.len(), 1);
    assert_eq!(result.why[0].edge_type, "derived_from");
    assert_eq!(result.why[0].to_memory_id, Some(2));
    assert_eq!(result.why[0].evidence_event_ids, vec![11, 12]);
    assert_eq!(result.why[0].source_candidate_id, Some(20));
    assert_eq!(result.why[0].source_operation_id, Some(30));
    Ok(())
}

#[test]
fn edge_details_scope_to_resolved_state_key() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (11, 'repo', '/repo', 'decision', 'other-deploy-target',
                 'other deploy target', 'active', NULL, 1700000000, 1700000010)",
        [],
    )?;
    insert_current_state_memory(
        &conn,
        1,
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory(
        &conn,
        2,
        "Deploy target",
        "Use production.",
        "active",
        10,
        Some(200),
        None,
    )?;
    insert_current_state_memory(
        &conn,
        3,
        "Other deploy target",
        "Use canary.",
        "stale",
        11,
        Some(100),
        Some(200),
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('supersedes', 1, 2, 10, '[7]', 'same state', 300)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('merged_into', 3, 2, 11, '[8]', 'different state', 310)",
        [],
    )?;

    let result = current_state(&conn, &request())?;

    assert_eq!(
        result
            .history
            .iter()
            .map(|memory| memory.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        result
            .why
            .iter()
            .map(|edge| edge.reason.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("same state")]
    );
    Ok(())
}

#[test]
fn as_of_why_excludes_edges_created_after_cutoff() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        1,
        "/repo",
        "Old deploy target",
        "Use staging.",
        "stale",
        10,
        100,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        200,
        Some(200),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('derived_from', NULL, 1, 10, '[1]', 'original candidate', 120)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('conflicts', 1, 2, 10, '[2]', 'future conflict', 180)",
        [],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(150),
            ..request()
        },
    )?;

    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(1));
    assert_eq!(
        result
            .why
            .iter()
            .map(|edge| edge.edge_type.as_str())
            .collect::<Vec<_>>(),
        vec!["derived_from"]
    );
    assert_eq!(result.why[0].evidence_event_ids, vec![1]);
    Ok(())
}

#[test]
fn as_of_history_excludes_edges_created_after_cutoff() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    insert_current_state_memory_at(
        &conn,
        3,
        "/repo",
        "Future merged target",
        "Use canary.",
        "stale",
        10,
        130,
        Some(130),
        Some(150),
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          reason, created_at_epoch)
         VALUES ('merged_into', 3, 2, 10, '[9]', 'future merge', 150)",
        [],
    )?;

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(120),
            ..request()
        },
    )?;

    assert_eq!(result.current.as_ref().map(|memory| memory.id), Some(2));
    assert!(result.history.is_empty());
    Ok(())
}

#[test]
fn as_of_facts_exclude_facts_learned_after_cutoff() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    for (id, object, learned_at) in [(1_i64, "known", 110_i64), (2_i64, "future", 150_i64)] {
        conn.execute(
            "INSERT INTO memory_facts
             (id, project, subject, predicate, object, valid_from_epoch,
              valid_to_epoch, learned_at_epoch, source_memory_id, source_event_ids,
              confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, '/repo', 'deploy-target', 'affects_project', ?2, 100,
                     NULL, ?3, 2, '[]', 0.9, 'active', ?3, ?3)",
            params![id, object, learned_at],
        )?;
    }

    let result = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(120),
            ..request()
        },
    )?;

    assert_eq!(
        result
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["known"]
    );
    Ok(())
}

#[test]
fn current_facts_exclude_invalidated_rows_even_when_status_is_active() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch,
          valid_to_epoch, learned_at_epoch, source_memory_id, source_event_ids,
          confidence, status, invalidated_at_epoch, created_at_epoch, updated_at_epoch)
         VALUES
          (1, '/repo', 'deploy-target', 'affects_project', 'staging', 100,
           NULL, 110, 2, '[]', 0.9, 'active', 150, 110, 150),
          (2, '/repo', 'deploy-target', 'affects_project', 'production', 160,
           NULL, 160, 2, '[]', 0.9, 'active', NULL, 160, 160)",
        [],
    )?;

    let current = current_state(&conn, &request())?;
    assert_eq!(
        current
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["production"]
    );

    let before_invalidation = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(120),
            ..request()
        },
    )?;
    assert_eq!(
        before_invalidation
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["staging"]
    );
    Ok(())
}

#[test]
fn as_of_facts_keep_backdated_superseded_fact_until_invalidation_time() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch,
          valid_to_epoch, learned_at_epoch, source_memory_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES
          (1, '/repo', 'deploy-target', 'affects_project', 'staging', 100,
           200, 110, 2, '[]', 0.9, NULL, 'stale', 250, 110, 250),
          (2, '/repo', 'deploy-target', 'affects_project', 'production', 200,
           NULL, 250, 2, '[]', 0.9, 1, 'active', NULL, 250, 250)",
        [],
    )?;

    let before_invalidation = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(225),
            ..request()
        },
    )?;
    assert_eq!(
        before_invalidation
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["staging"]
    );

    let after_invalidation = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(260),
            ..request()
        },
    )?;
    assert_eq!(
        after_invalidation
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["production"]
    );
    Ok(())
}

#[test]
fn as_of_facts_use_known_replacement_without_returning_superseded_fact() -> Result<()> {
    let conn = current_state_test_conn()?;
    insert_state_key(&conn)?;
    insert_current_state_memory_at(
        &conn,
        2,
        "/repo",
        "Deploy target",
        "Use production.",
        "active",
        10,
        100,
        Some(100),
        None,
    )?;
    set_current_memory(&conn, 2)?;
    conn.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch,
          valid_to_epoch, learned_at_epoch, source_memory_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES
          (1, '/repo', 'deploy-target', 'affects_project', 'staging', 100,
           200, 110, 2, '[]', 0.9, NULL, 'stale', 250, 110, 250),
          (2, '/repo', 'deploy-target', 'affects_project', 'production', 200,
           NULL, 200, 2, '[]', 0.9, 1, 'active', NULL, 250, 250)",
        [],
    )?;

    let as_of = current_state(
        &conn,
        &CurrentStateRequest {
            as_of_epoch: Some(225),
            ..request()
        },
    )?;
    assert_eq!(
        as_of
            .facts
            .iter()
            .map(|fact| fact.object.as_str())
            .collect::<Vec<_>>(),
        vec!["production"]
    );
    Ok(())
}
