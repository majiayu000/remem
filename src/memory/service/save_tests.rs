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
