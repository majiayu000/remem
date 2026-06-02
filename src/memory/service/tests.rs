use super::{resolve_local_note_path, sanitize_segment, save_memory, SaveMemoryRequest};
use crate::db::{self, test_support::ScopedTestDataDir};
#[cfg(unix)]
use std::os::unix::fs::symlink;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn sanitize_segment_falls_back_for_empty_slug() {
    let got = sanitize_segment("!!!", "fallback", 64);
    assert_eq!(got, "fallback");
}

// --- path confinement tests ---

#[test]
fn resolve_absolute_path_inside_base_is_allowed() {
    let _dir = ScopedTestDataDir::new("path-inside");
    let base = crate::db::data_dir();
    let target = base.join("notes").join("test.md");
    let got = resolve_local_note_path("proj", Some("title"), Some(target.to_str().unwrap()));
    assert!(got.is_ok(), "path inside base should be allowed: {:?}", got);
}

#[test]
fn resolve_absolute_path_outside_base_is_rejected() {
    let _dir = ScopedTestDataDir::new("path-outside");
    let got = resolve_local_note_path("proj", Some("title"), Some("/etc/passwd"));
    assert!(
        got.is_err(),
        "absolute path outside base should be rejected"
    );
    assert!(got
        .unwrap_err()
        .to_string()
        .contains("outside the allowed directory"));
}

#[test]
fn resolve_relative_traversal_is_rejected() {
    let _dir = ScopedTestDataDir::new("path-traversal");
    let got = resolve_local_note_path("proj", Some("title"), Some("../../etc/passwd"));
    assert!(got.is_err(), "path traversal should be rejected");
    assert!(got
        .unwrap_err()
        .to_string()
        .contains("outside the allowed directory"));
}

#[test]
fn resolve_tilde_path_is_rejected() {
    let _dir = ScopedTestDataDir::new("path-tilde");
    let got = resolve_local_note_path("proj", Some("title"), Some("~/.ssh/authorized_keys"));
    assert!(got.is_err(), "tilde path should be rejected (not expanded)");
}

#[test]
fn save_memory_preference_defaults_to_project_scope() {
    let _dir = ScopedTestDataDir::new("preference-default-project-scope");
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text: "Prefer project-specific workflow notes".to_string(),
        title: Some("Preference".to_string()),
        project: Some("proj".to_string()),
        memory_type: Some("preference".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req).expect("preference save should succeed");
    let scope: String = conn
        .query_row(
            "SELECT scope FROM memories WHERE id = ?1",
            [saved.id],
            |row| row.get(0),
        )
        .expect("scope query should succeed");
    assert_eq!(scope, "project");
}

#[test]
fn save_memory_insert_reports_durable_details() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-insert-feedback");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Remember the durable feedback shape.".to_string(),
        title: Some("Durable feedback".to_string()),
        project: Some("proj".to_string()),
        memory_type: Some("decision".to_string()),
        branch: Some("main".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req)?;

    assert_eq!(saved.status, "saved");
    assert_eq!(saved.operation, "add");
    assert!(!saved.upserted);
    assert_eq!(saved.project, "proj");
    assert_eq!(saved.scope, "project");
    assert_eq!(saved.topic_key, None);
    assert_eq!(saved.branch.as_deref(), Some("main"));
    assert_eq!(saved.local_copy.status, "disabled");
    assert_eq!(saved.local_status, "disabled");
    assert_eq!(saved.local_path, None);
    assert_eq!(saved.next_step.tool, "get_observations");
    assert_eq!(saved.next_step.ids, vec![saved.id]);
    assert_eq!(saved.next_step.source, "memory");
    assert!(saved.created_at_epoch > 0);
    assert!(saved.updated_at_epoch > 0);
    let logged_operation: String = conn.query_row(
        "SELECT operation FROM memory_operation_log WHERE result_memory_id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;
    assert_eq!(logged_operation, "add");
    Ok(())
}

#[test]
fn save_memory_topic_key_update_reports_updated_operation() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-topic-update-feedback");
    let conn = db::open_db()?;
    let first_req = SaveMemoryRequest {
        text: "First body".to_string(),
        title: Some("Topic feedback".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("durable-feedback".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let first = save_memory(&conn, &first_req)?;
    assert_eq!(first.operation, "add");
    assert!(first.upserted);

    let second_req = SaveMemoryRequest {
        text: "Updated body".to_string(),
        title: Some("Topic feedback updated".to_string()),
        ..first_req
    };
    let second = save_memory(&conn, &second_req)?;

    assert_eq!(second.id, first.id);
    assert_eq!(second.operation, "update");
    assert!(second.upserted);
    assert_eq!(second.topic_key.as_deref(), Some("durable-feedback"));
    let operations = conn
        .prepare("SELECT operation FROM memory_operation_log ORDER BY id ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(operations, vec!["add".to_string(), "update".to_string()]);
    Ok(())
}

#[test]
fn save_memory_repeated_same_fact_reports_noop_operation() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-topic-noop-feedback");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "This fact is already represented.".to_string(),
        title: Some("Represented fact".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("represented-fact".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let first = save_memory(&conn, &req)?;
    let second = save_memory(&conn, &req)?;

    assert_eq!(second.id, first.id);
    assert_eq!(second.operation, "noop");
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let (operation, noop_reason): (String, Option<String>) = conn.query_row(
        "SELECT operation, noop_reason
         FROM memory_operation_log
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(memory_count, 1);
    assert_eq!(operation, "noop");
    assert_eq!(
        noop_reason.as_deref(),
        Some("already represented by active memory")
    );
    Ok(())
}

#[test]
fn save_memory_disabled_local_copy_reports_structured_reason() {
    let _dir = ScopedTestDataDir::new("save-disabled-local-copy-feedback");
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text: "No local copy for this save.".to_string(),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req).expect("save should succeed");

    assert_eq!(saved.local_copy.status, "disabled");
    assert_eq!(saved.local_copy.path, None);
    assert!(saved
        .local_copy
        .reason
        .as_deref()
        .is_some_and(|reason| { reason.contains("disabled") }));
    assert_eq!(saved.local_status, saved.local_copy.status);
    assert_eq!(saved.local_path, saved.local_copy.path);
}

#[test]
fn save_memory_local_path_override_reports_structured_path() {
    let test_dir = ScopedTestDataDir::new("save-local-path-feedback");
    let conn = db::open_db().expect("db should open");
    let local_path = test_dir
        .path
        .join("manual-notes")
        .join("proj")
        .join("custom-note.md");
    let req = SaveMemoryRequest {
        text: "Local path override body".to_string(),
        title: Some("Local path override".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req).expect("save should succeed");

    assert_eq!(saved.local_copy.status, "saved");
    let canonical_local_path = local_path
        .canonicalize()
        .expect("saved local path should canonicalize");
    assert_eq!(
        saved.local_copy.path.as_deref(),
        Some(canonical_local_path.to_str().expect("utf8 local path"))
    );
    assert_eq!(saved.local_copy.reason, None);
    assert_eq!(saved.local_status, "saved");
    assert_eq!(saved.local_path, saved.local_copy.path);
    assert!(local_path.exists());
}

#[test]
fn save_memory_lesson_creates_lesson_metadata() {
    let _dir = ScopedTestDataDir::new("lesson-save-metadata");
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text:
            "Lesson: route generic lesson saves through the lesson writer so context can load them."
                .to_string(),
        title: Some("Lesson metadata".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("lesson-save-metadata".to_string()),
        memory_type: Some("lesson".to_string()),
        files: Some(vec!["src/memory/service/save.rs".to_string()]),
        branch: Some("main".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req).expect("lesson save should succeed");

    let (metadata_count, files, branch): (i64, String, String) = conn
        .query_row(
            "SELECT COUNT(l.memory_id), m.files, m.branch
             FROM memories m
             LEFT JOIN memory_lessons l ON l.memory_id = m.id
             WHERE m.id = ?1",
            [saved.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("lesson metadata query should succeed");
    assert_eq!(metadata_count, 1);
    assert_eq!(branch, "main");
    assert!(files.contains("src/memory/service/save.rs"));
}

#[test]
fn save_memory_outside_local_path_does_not_persist_memory() {
    let _dir = ScopedTestDataDir::new("save-outside-path-no-db-write");
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some("/etc/passwd".to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let err = save_memory(&conn, &req).expect_err("out-of-bounds local_path should fail");

    assert!(
        err.to_string().contains("outside the allowed directory"),
        "unexpected error: {err:?}"
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count query should succeed");
    assert_eq!(memory_count, 0, "db should not persist a memory row");
}

#[test]
fn save_memory_local_write_failure_does_not_persist_memory() {
    let test_dir = ScopedTestDataDir::new("save-local-write-failure-no-db-write");
    let conn = db::open_db().expect("db should open");
    let blocking_file = test_dir.path.join("manual-notes").join("proj");
    std::fs::create_dir_all(blocking_file.parent().expect("blocking file parent"))
        .expect("create blocking file parent");
    std::fs::write(&blocking_file, "not a directory").expect("create blocking file");

    let local_path = blocking_file.join("forced-failure.md");
    let req = SaveMemoryRequest {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let err = save_memory(&conn, &req).expect_err("local write should abort save");

    assert!(
        err.to_string().contains("Not a directory")
            || err.to_string().contains("not a directory")
            || err.to_string().contains("File exists"),
        "unexpected error: {err:?}"
    );
    assert!(
        !local_path.exists(),
        "local copy path should not exist after a write failure: {:?}",
        local_path
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count query should succeed");
    assert_eq!(memory_count, 0, "db should not persist a memory row");
}

#[test]
fn save_memory_db_failure_does_not_leave_local_copy_behind() {
    let test_dir = ScopedTestDataDir::new("save-db-failure-no-local-copy");
    let conn = db::open_db().expect("db should open");
    conn.execute_batch(
        "CREATE TRIGGER fail_memory_insert BEFORE INSERT ON memories BEGIN
            SELECT RAISE(ABORT, 'forced insert failure');
        END;",
    )
    .expect("failure trigger should be created");

    let local_path = test_dir
        .path
        .join("manual-notes")
        .join("proj")
        .join("forced-failure.md");
    let req = SaveMemoryRequest {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let err = save_memory(&conn, &req).expect_err("insert trigger should abort save");

    assert!(
        err.to_string().contains("forced insert failure"),
        "unexpected error: {err:?}"
    );
    assert!(
        !local_path.exists(),
        "local copy should not be written when db insert fails: {:?}",
        local_path
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count query should succeed");
    assert_eq!(memory_count, 0, "db should not persist a memory row");
}

#[test]
fn save_memory_db_failure_restores_existing_local_copy() {
    let test_dir = ScopedTestDataDir::new("save-db-failure-restores-existing-local-copy");
    let conn = db::open_db().expect("db should open");
    conn.execute_batch(
        "CREATE TRIGGER fail_memory_insert BEFORE INSERT ON memories BEGIN
            SELECT RAISE(ABORT, 'forced insert failure');
        END;",
    )
    .expect("failure trigger should be created");

    let local_path = test_dir
        .path
        .join("manual-notes")
        .join("proj")
        .join("existing-note.md");
    std::fs::create_dir_all(local_path.parent().expect("existing note parent"))
        .expect("create existing note parent");
    std::fs::write(&local_path, "original note body").expect("seed existing note");

    let req = SaveMemoryRequest {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let err = save_memory(&conn, &req).expect_err("insert trigger should abort save");

    assert!(
        err.to_string().contains("forced insert failure"),
        "unexpected error: {err:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&local_path).expect("existing note should remain readable"),
        "original note body",
        "db failure should restore the prior local note contents"
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count query should succeed");
    assert_eq!(memory_count, 0, "db should not persist a memory row");
}

#[test]
fn save_memory_existing_directory_local_path_does_not_persist_memory() {
    let test_dir = ScopedTestDataDir::new("save-directory-local-path-rejected");
    let conn = db::open_db().expect("db should open");

    let local_path = test_dir
        .path
        .join("manual-notes")
        .join("proj")
        .join("existing-dir");
    let nested_entry = local_path.join("nested.txt");
    std::fs::create_dir_all(&local_path).expect("create existing directory local path");
    std::fs::write(&nested_entry, "keep me").expect("seed nested entry");

    let req = SaveMemoryRequest {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let err = save_memory(&conn, &req).expect_err("directory local_path should fail");

    assert!(
        err.to_string()
            .contains("must reference a file, not a directory"),
        "unexpected error: {err:?}"
    );
    assert!(
        local_path.is_dir(),
        "directory path should remain a directory"
    );
    assert_eq!(
        std::fs::read_to_string(&nested_entry).expect("nested entry should stay intact"),
        "keep me"
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count query should succeed");
    assert_eq!(memory_count, 0, "db should not persist a memory row");
}

#[cfg(unix)]
#[test]
fn save_memory_db_failure_restores_write_only_existing_local_copy() {
    let test_dir = ScopedTestDataDir::new("save-db-failure-restores-write-only-local-copy");
    let conn = db::open_db().expect("db should open");
    conn.execute_batch(
        "CREATE TRIGGER fail_memory_insert BEFORE INSERT ON memories BEGIN
            SELECT RAISE(ABORT, 'forced insert failure');
        END;",
    )
    .expect("failure trigger should be created");

    let local_path = test_dir
        .path
        .join("manual-notes")
        .join("proj")
        .join("write-only-note.md");
    std::fs::create_dir_all(local_path.parent().expect("existing note parent"))
        .expect("create existing note parent");
    std::fs::write(&local_path, "original note body").expect("seed existing note");

    let mut permissions = std::fs::metadata(&local_path)
        .expect("read existing permissions")
        .permissions();
    permissions.set_mode(0o200);
    std::fs::set_permissions(&local_path, permissions).expect("make existing note write-only");

    let req = SaveMemoryRequest {
        text: "body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let err = save_memory(&conn, &req).expect_err("insert trigger should abort save");

    assert!(
        err.to_string().contains("forced insert failure"),
        "unexpected error: {err:?}"
    );

    let mut readable_permissions = std::fs::metadata(&local_path)
        .expect("restored note should exist")
        .permissions();
    readable_permissions.set_mode(0o600);
    std::fs::set_permissions(&local_path, readable_permissions)
        .expect("make restored note readable");

    assert_eq!(
        std::fs::read_to_string(&local_path).expect("restored note should be readable"),
        "original note body",
        "db failure should restore the prior local note contents"
    );

    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count query should succeed");
    assert_eq!(memory_count, 0, "db should not persist a memory row");
}

#[cfg(unix)]
#[test]
fn save_memory_existing_symlink_local_path_stays_a_symlink() {
    let test_dir = ScopedTestDataDir::new("save-symlink-local-path-preserved");
    let conn = db::open_db().expect("db should open");

    let project_dir = test_dir.path.join("manual-notes").join("proj");
    std::fs::create_dir_all(&project_dir).expect("create project dir");

    let target_path = project_dir.join("target-note.md");
    std::fs::write(&target_path, "original note body").expect("seed symlink target");

    let local_path = project_dir.join("symlink-note.md");
    symlink(&target_path, &local_path).expect("create local note symlink");

    let req = SaveMemoryRequest {
        text: "updated body".to_string(),
        title: Some("Memory".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        ..SaveMemoryRequest::default()
    };

    let saved = save_memory(&conn, &req).expect("save through symlink should succeed");

    assert_eq!(saved.status, "saved");
    assert!(
        std::fs::symlink_metadata(&local_path)
            .expect("local path metadata")
            .file_type()
            .is_symlink(),
        "local path should remain a symlink"
    );

    let symlink_target = std::fs::read_link(&local_path).expect("read symlink target");
    assert_eq!(
        symlink_target, target_path,
        "symlink target should be preserved"
    );

    let updated = std::fs::read_to_string(&target_path).expect("read updated target");
    assert!(
        updated.contains("updated body"),
        "saved note should be written through the symlink target: {updated}"
    );
}

#[test]
fn resolve_base_dir_itself_is_rejected() {
    let _dir = ScopedTestDataDir::new("path-base-itself");
    let base = crate::db::data_dir();
    let got = resolve_local_note_path("proj", Some("title"), Some(base.to_str().unwrap()));
    assert!(
        got.is_err(),
        "base dir itself should be rejected — must be a file inside base"
    );
}

#[test]
fn resolve_none_local_path_returns_default() {
    let _dir = ScopedTestDataDir::new("path-default");
    unsafe { std::env::remove_var("REMEM_SAVE_MEMORY_LOCAL_DIR") };

    let got = resolve_local_note_path("proj", Some("title"), None);
    assert!(got.is_ok());
    let path = got.unwrap();
    assert!(path.is_absolute());
    let base = crate::db::data_dir();
    assert!(
        path.starts_with(&base),
        "default path {:?} should be inside {:?}",
        path,
        base
    );
}

#[test]
fn resolve_none_local_path_allows_env_directory_inside_base() {
    let _dir = ScopedTestDataDir::new("path-default-env-inside");
    let base = crate::db::data_dir();
    let env_dir = base.join("manual-notes-custom");
    unsafe { std::env::set_var("REMEM_SAVE_MEMORY_LOCAL_DIR", &env_dir) };

    let got = resolve_local_note_path("proj", Some("title"), None);
    unsafe { std::env::remove_var("REMEM_SAVE_MEMORY_LOCAL_DIR") };

    assert!(
        got.is_ok(),
        "env path inside base should be allowed: {got:?}"
    );
    let path = got.unwrap();
    assert!(
        path.starts_with(&env_dir),
        "default path {:?} should be inside env dir {:?}",
        path,
        env_dir
    );
}

#[test]
fn resolve_none_local_path_rejects_env_directory_outside_base() {
    let _dir = ScopedTestDataDir::new("path-default-env-outside");
    let outside = std::env::temp_dir().join("remem-outside-manual-notes");
    unsafe { std::env::set_var("REMEM_SAVE_MEMORY_LOCAL_DIR", &outside) };

    let got = resolve_local_note_path("proj", Some("title"), None);
    unsafe { std::env::remove_var("REMEM_SAVE_MEMORY_LOCAL_DIR") };

    assert!(
        got.is_err(),
        "env path outside base should be rejected instead of bypassing confinement"
    );
    assert!(got
        .unwrap_err()
        .to_string()
        .contains("outside the allowed directory"));
}

#[test]
fn resolve_empty_local_path_uses_confined_default() {
    let _dir = ScopedTestDataDir::new("path-default-empty");
    let outside = std::env::temp_dir().join("remem-outside-empty-manual-notes");
    unsafe { std::env::set_var("REMEM_SAVE_MEMORY_LOCAL_DIR", &outside) };

    let got = resolve_local_note_path("proj", Some("title"), Some("   "));
    unsafe { std::env::remove_var("REMEM_SAVE_MEMORY_LOCAL_DIR") };

    assert!(
        got.is_err(),
        "empty local_path should follow the same confined default branch"
    );
}

#[test]
fn resolve_none_local_path_allows_relative_remem_data_dir_default() {
    let _guard = ScopedTestDataDir::new("path-default-relative-data-dir");
    let original_cwd = std::env::current_dir().expect("read cwd");
    let temp_root = std::env::temp_dir().join(format!(
        "remem-relative-data-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ));
    let project_root = temp_root.join("workspace");
    std::fs::create_dir_all(&project_root).expect("create project root");
    let expected_base = project_root
        .canonicalize()
        .expect("canonicalize project root")
        .join(".remem");
    unsafe {
        std::env::set_current_dir(&project_root).expect("enter project root");
        std::env::set_var("REMEM_DATA_DIR", ".remem");
        std::env::remove_var("REMEM_SAVE_MEMORY_LOCAL_DIR");
    }

    let got = resolve_local_note_path("proj", Some("title"), None);

    unsafe {
        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        std::env::remove_var("REMEM_DATA_DIR");
    }
    let _ = std::fs::remove_dir_all(&temp_root);

    assert!(
        got.is_ok(),
        "relative REMEM_DATA_DIR default path should be allowed: {got:?}"
    );
    let path = got.unwrap();
    assert!(
        path.is_absolute(),
        "resolved path should be absolute: {path:?}"
    );
    assert!(
        path.starts_with(&expected_base),
        "resolved path {path:?} should stay inside {:?}",
        expected_base
    );
}

#[test]
fn resolve_none_local_path_allows_relative_remem_data_dir_with_parent_segments() {
    let _guard = ScopedTestDataDir::new("path-default-relative-data-dir-parent-segments");
    let original_cwd = std::env::current_dir().expect("read cwd");
    let temp_root = std::env::temp_dir().join(format!(
        "remem-relative-parent-data-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ));
    let workspace_root = temp_root.join("workspace");
    let project_root = workspace_root.join("project");
    std::fs::create_dir_all(&project_root).expect("create project root");
    let expected_base = workspace_root
        .canonicalize()
        .expect("canonicalize workspace root")
        .join(".remem");
    unsafe {
        std::env::set_current_dir(&project_root).expect("enter project root");
        std::env::set_var("REMEM_DATA_DIR", "../.remem");
        std::env::remove_var("REMEM_SAVE_MEMORY_LOCAL_DIR");
    }

    let got = resolve_local_note_path("proj", Some("title"), None);

    unsafe {
        std::env::set_current_dir(&original_cwd).expect("restore cwd");
        std::env::remove_var("REMEM_DATA_DIR");
    }
    let _ = std::fs::remove_dir_all(&temp_root);

    assert!(
        got.is_ok(),
        "relative REMEM_DATA_DIR with parent segments should be allowed: {got:?}"
    );
    let path = got.unwrap();
    assert!(
        path.starts_with(&expected_base),
        "resolved path {path:?} should stay inside {:?}",
        expected_base
    );
}
