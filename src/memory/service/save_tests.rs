use super::{save_memory, SaveMemoryRequest};
use crate::db::{self, test_support::ScopedTestDataDir};

#[test]
fn repeated_lesson_save_reinforces_metadata_and_logs_update() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("lesson-save-reinforces");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Lesson: keep operation audit without losing lesson reinforcement.".to_string(),
        title: Some("Lesson reinforcement".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("lesson-reinforcement".to_string()),
        memory_type: Some("lesson".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let first = save_memory(&conn, &req)?;
    let second = save_memory(&conn, &req)?;

    assert_eq!(first.id, second.id);
    assert_eq!(second.operation, "update");
    let reinforcement_count: i64 = conn.query_row(
        "SELECT reinforcement_count FROM memory_lessons WHERE memory_id = ?1",
        [first.id],
        |row| row.get(0),
    )?;
    let operations = conn
        .prepare("SELECT operation FROM memory_operation_log ORDER BY id ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(reinforcement_count, 2);
    assert_eq!(operations, vec!["add".to_string(), "update".to_string()]);
    Ok(())
}

#[test]
fn semantic_preference_duplicate_logs_update() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("semantic-preference-duplicate-update");
    let conn = db::open_db()?;
    let first = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Prefer small reversible changes and include verification output for every fix."
                .to_string(),
            title: Some("Small reversible changes".to_string()),
            project: Some("proj".to_string()),
            topic_key: Some("preference-a1b2c3d4".to_string()),
            memory_type: Some("preference".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    let second = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Prefer small reversible code changes with verification output for each fix."
                .to_string(),
            title: Some("Reversible verified changes".to_string()),
            project: Some("proj".to_string()),
            topic_key: Some("preference-deadbeef".to_string()),
            memory_type: Some("preference".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;

    assert_eq!(second.id, first.id);
    assert_eq!(second.operation, "update");
    let logs = conn
        .prepare("SELECT operation, reason FROM memory_operation_log ORDER BY id ASC")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(
        logs.iter()
            .map(|(operation, _)| operation.as_str())
            .collect::<Vec<_>>(),
        vec!["add", "update"]
    );
    assert!(
        logs[1]
            .1
            .contains("semantic duplicate memory will be updated"),
        "semantic update should be auditable, got: {}",
        logs[1].1
    );
    Ok(())
}

#[test]
fn direct_save_refreshes_expired_same_text_current_fact() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-expired-current-fact-refresh");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Local dev server is currently running at localhost:5173.".to_string(),
        title: Some("Dev server status".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("repo:proj:dev-server".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let first = save_memory(&conn, &req)?;
    conn.execute(
        "UPDATE memories
         SET expires_at_epoch = ?1, valid_from_epoch = ?2
         WHERE id = ?3",
        rusqlite::params![1_i64, 0_i64, first.id],
    )?;

    let second = save_memory(&conn, &req)?;

    assert_eq!(second.id, first.id);
    assert_eq!(second.operation, "update");
    let (status, expires_at): (String, i64) = conn.query_row(
        "SELECT status, expires_at_epoch FROM memories WHERE id = ?1",
        [first.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "active");
    assert!(
        expires_at > chrono::Utc::now().timestamp(),
        "same-text save should refresh expired currentness metadata"
    );
    let operation: String = conn.query_row(
        "SELECT operation FROM memory_operation_log ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(operation, "update");
    Ok(())
}

#[test]
fn direct_save_same_text_metadata_change_updates_row() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-same-text-metadata-update");
    let conn = db::open_db()?;
    let first_req = SaveMemoryRequest {
        text: "This fact keeps the same content while durable metadata changes.".to_string(),
        title: Some("Old title".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("metadata-refresh".to_string()),
        memory_type: Some("discovery".to_string()),
        files: Some(vec!["src/old.rs".to_string()]),
        branch: Some("main".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let first = save_memory(&conn, &first_req)?;
    let second_req = SaveMemoryRequest {
        title: Some("New title".to_string()),
        files: Some(vec!["src/new.rs".to_string()]),
        branch: Some("feature".to_string()),
        ..first_req
    };

    let second = save_memory(&conn, &second_req)?;

    assert_eq!(second.id, first.id);
    assert_eq!(second.operation, "update");
    let (title, files, branch): (String, Option<String>, Option<String>) = conn.query_row(
        "SELECT title, files, branch FROM memories WHERE id = ?1",
        [first.id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(title, "New title");
    assert_eq!(files.as_deref(), Some("[\"src/new.rs\"]"));
    assert_eq!(branch.as_deref(), Some("feature"));
    Ok(())
}

#[test]
fn direct_save_updates_active_duplicate_topic_row() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-active-topic-duplicate-target");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Use the current active duplicate row for planner-aligned writes.".to_string(),
        title: Some("Planner target".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("planner-target".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let stale = save_memory(&conn, &req)?;
    conn.execute(
        "UPDATE memories
         SET status = 'stale', updated_at_epoch = ?1
         WHERE id = ?2",
        rusqlite::params![10_i64, stale.id],
    )?;
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, status, branch, scope,
          source_project, target_project, owner_scope, owner_key, context_class)
         VALUES (?1, 'proj', 'planner-target', 'Current active row',
                 'Existing active duplicate content.', 'discovery', NULL,
                 'Existing active duplicate content.', ?2, ?3, 'active', NULL, 'project',
                 'proj', 'proj', 'repo', 'proj', 'startup_core')",
        rusqlite::params!["legacy-active-session", 20_i64, 20_i64],
    )?;
    let active = conn.last_insert_rowid();
    let update_req = SaveMemoryRequest {
        text: "Updated planner-aligned content.".to_string(),
        title: Some("Planner target updated".to_string()),
        ..req
    };

    let saved = save_memory(&conn, &update_req)?;

    assert_eq!(saved.id, active);
    assert_eq!(saved.operation, "update");
    let stale_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [stale.id],
        |row| row.get(0),
    )?;
    assert_eq!(stale_status, "stale");
    let active_content: String = conn.query_row(
        "SELECT content FROM memories WHERE id = ?1",
        [active],
        |row| row.get(0),
    )?;
    assert_eq!(active_content, "Updated planner-aligned content.");
    let active_topic_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE project = 'proj'
           AND topic_key = 'planner-target'
           AND scope = 'project'
           AND status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_topic_rows, 1);
    let logged_result: i64 = conn.query_row(
        "SELECT result_memory_id FROM memory_operation_log ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(logged_result, active);
    Ok(())
}

#[test]
fn save_memory_creates_manual_claim_after_successful_memory_write() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-memory-claim-success");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Use exact session memory claims to suppress duplicate summary candidates."
            .to_string(),
        title: Some("Session claims".to_string()),
        project: Some("proj".to_string()),
        session_id: Some("session-claim-success".to_string()),
        host: Some("codex-cli".to_string()),
        memory_type: Some("decision".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req)?;

    assert_eq!(saved.claim_status, "saved");
    let claim_id = saved
        .claim_id
        .ok_or_else(|| anyhow::anyhow!("claim id should be returned"))?;
    assert_eq!(saved.claim_error, None);
    let claim: (i64, String, String, String, String, String) = conn.query_row(
        "SELECT memory_id, project, session_id, host, claim_source, memory_type
         FROM memory_claims
         WHERE id = ?1",
        [claim_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    assert_eq!(claim.0, saved.id);
    assert_eq!(claim.1, "proj");
    assert_eq!(claim.2, "session-claim-success");
    assert_eq!(claim.3, "codex-cli");
    assert_eq!(claim.4, "manual_save");
    assert_eq!(claim.5, "decision");
    Ok(())
}

#[test]
fn save_memory_claim_disabled_preserves_existing_behavior() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-memory-claim-disabled");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Claim disabled should still save durable memory.".to_string(),
        title: Some("Claim disabled".to_string()),
        project: Some("proj".to_string()),
        session_id: Some("session-claim-disabled".to_string()),
        memory_type: Some("discovery".to_string()),
        local_copy_enabled: Some(false),
        claim_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req)?;

    assert_eq!(saved.status, "saved");
    assert_eq!(saved.claim_status, "disabled");
    assert_eq!(saved.claim_id, None);
    assert_eq!(saved.claim_error, None);
    let claim_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_claims", [], |row| row.get(0))?;
    assert_eq!(claim_count, 0);
    Ok(())
}

#[test]
fn save_memory_claim_write_failure_is_reported_not_silent() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-memory-claim-failure");
    let conn = db::open_db()?;
    conn.execute("DROP TABLE memory_claims", [])?;
    let req = SaveMemoryRequest {
        text: "Durable memory should survive a claim write failure.".to_string(),
        title: Some("Claim failure".to_string()),
        project: Some("proj".to_string()),
        session_id: Some("session-claim-failure".to_string()),
        memory_type: Some("discovery".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req)?;

    assert_eq!(saved.status, "saved");
    assert_eq!(saved.claim_status, "failed");
    assert_eq!(saved.claim_id, None);
    assert!(saved
        .claim_error
        .as_deref()
        .is_some_and(|error| error.contains("memory_claims")));
    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 1);
    Ok(())
}
