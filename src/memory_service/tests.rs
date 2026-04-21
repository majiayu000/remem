use super::{resolve_local_note_path, sanitize_segment, save_memory, SaveMemoryRequest};
use crate::db::{self, test_support::ScopedTestDataDir};

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
