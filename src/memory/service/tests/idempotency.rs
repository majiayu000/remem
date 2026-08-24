use super::*;

struct EnvVarRestore {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarRestore {
    fn capture(key: &'static str) -> Self {
        Self {
            key,
            previous: std::env::var_os(key),
        }
    }
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[test]
fn idempotent_replay_performs_the_reported_local_copy_side_effect() {
    let test_dir = ScopedTestDataDir::new("save-local-copy-idempotent-replay");
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text: "Idempotent replay must not report an unwritten local copy.".to_string(),
        title: Some("Idempotent local copy".to_string()),
        project: Some("proj".to_string()),
        local_copy_enabled: Some(true),
        claim_enabled: Some(false),
        idempotency_key: Some("local-copy-replay".to_string()),
        ..SaveMemoryRequest::default()
    };

    let first = save_memory(&conn, &req).expect("first save should succeed");
    let local_path = std::path::PathBuf::from(
        first
            .local_copy
            .path
            .as_deref()
            .expect("first save should report its default path"),
    );
    let original_content = std::fs::read(&local_path).expect("read the first local copy");
    std::fs::remove_file(&local_path).expect("remove the first local copy");
    std::thread::sleep(std::time::Duration::from_secs(1));
    let replay = save_memory(&conn, &req).expect("receipt replay should succeed");

    assert_eq!(replay.id, first.id);
    assert_eq!(replay.operation, "noop");
    assert_eq!(replay.local_copy.status, "saved");
    assert_eq!(replay.local_copy.path, first.local_copy.path);
    assert!(local_path.exists(), "reported replay path must be written");
    assert_eq!(
        std::fs::read(&local_path).expect("read repaired local copy"),
        original_content,
        "replay must restore the exact original rendered artifact"
    );
    drop(test_dir);
}

#[cfg(unix)]
#[test]
fn idempotent_replay_reconstructs_missing_parent_directories() {
    let test_dir = ScopedTestDataDir::new("save-local-copy-parent-replay");
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text: "Replay must safely reconstruct its exact local-copy parent.".to_string(),
        title: Some("Missing parent replay".to_string()),
        project: Some("proj".to_string()),
        local_copy_enabled: Some(true),
        claim_enabled: Some(false),
        idempotency_key: Some("local-copy-parent-replay".to_string()),
        ..SaveMemoryRequest::default()
    };

    let first = save_memory(&conn, &req).expect("first save should succeed");
    let local_path = std::path::PathBuf::from(
        first
            .local_copy
            .path
            .as_deref()
            .expect("first save should report its local path"),
    );
    let original_content = std::fs::read(&local_path).expect("read first local copy");
    std::fs::remove_dir_all(
        local_path
            .parent()
            .expect("local copy should have a parent"),
    )
    .expect("remove local-copy parent");

    let replay = save_memory(&conn, &req).expect("receipt replay should reconstruct parent");

    assert_eq!(replay.local_copy.path, first.local_copy.path);
    assert_eq!(
        std::fs::read(&local_path).expect("read reconstructed local copy"),
        original_content
    );
    drop(test_dir);
}

#[test]
fn idempotent_replay_uses_disabled_receipt_after_environment_toggle() {
    let _test_dir = ScopedTestDataDir::new("save-local-copy-disabled-replay");
    let _restore = EnvVarRestore::capture("REMEM_SAVE_MEMORY_LOCAL_COPY");
    unsafe { std::env::set_var("REMEM_SAVE_MEMORY_LOCAL_COPY", "0") };
    let conn = db::open_db().expect("db should open");
    let req = SaveMemoryRequest {
        text: "Replay must use the durable disabled outcome.".to_string(),
        title: Some("Disabled local copy".to_string()),
        project: Some("proj".to_string()),
        claim_enabled: Some(false),
        idempotency_key: Some("local-copy-disabled-replay".to_string()),
        ..SaveMemoryRequest::default()
    };

    let first = save_memory(&conn, &req).expect("first save should succeed");
    assert_eq!(first.local_copy.status, "disabled");
    unsafe { std::env::set_var("REMEM_SAVE_MEMORY_LOCAL_COPY", "1") };

    let replay = save_memory(&conn, &req).expect("receipt replay should succeed");

    assert_eq!(replay.id, first.id);
    assert_eq!(replay.operation, "noop");
    assert_eq!(replay.local_copy.status, "disabled");
    assert_eq!(replay.local_copy.path, None);
}

#[cfg(unix)]
#[test]
fn idempotent_replay_rejects_leaf_symlink_drift() {
    let test_dir = ScopedTestDataDir::new("save-local-copy-symlink-replay");
    let conn = db::open_db().expect("db should open");
    let local_path = test_dir.path.join("notes").join("exact.md");
    let victim_path = test_dir.path.join("notes").join("victim.md");
    let req = SaveMemoryRequest {
        text: "Replay must not follow a newly introduced symlink.".to_string(),
        title: Some("Symlink-safe replay".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        claim_enabled: Some(false),
        idempotency_key: Some("local-copy-symlink-replay".to_string()),
        ..SaveMemoryRequest::default()
    };

    save_memory(&conn, &req).expect("first save should succeed");
    std::fs::remove_file(&local_path).expect("remove original local copy");
    std::fs::write(&victim_path, b"must remain unchanged").expect("write symlink target");
    symlink(&victim_path, &local_path).expect("replace receipt path with symlink");

    let error = save_memory(&conn, &req).expect_err("symlink drift must fail closed");

    assert!(
        error.to_string().contains("drifted") || error.to_string().contains("symlink"),
        "unexpected replay error: {error:#}"
    );
    assert_eq!(
        std::fs::read(&victim_path).expect("read symlink target"),
        b"must remain unchanged"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn save_rejects_non_utf8_canonical_local_copy_receipt_path() {
    use std::os::unix::ffi::OsStringExt;

    let test_dir = ScopedTestDataDir::new("save-local-copy-non-utf8-receipt");
    let conn = db::open_db().expect("db should open");
    let notes_dir = test_dir.path.join("notes");
    std::fs::create_dir_all(&notes_dir).expect("create notes directory");
    let invalid_target = notes_dir.join(std::ffi::OsString::from_vec(vec![
        b'n', 0xff, b'.', b'm', b'd',
    ]));
    let local_path = notes_dir.join("utf8-link.md");
    std::fs::write(&invalid_target, b"original").expect("write non-UTF-8 target");
    symlink(&invalid_target, &local_path).expect("create UTF-8 symlink");
    let req = SaveMemoryRequest {
        text: "A durable receipt must preserve its exact path.".to_string(),
        title: Some("Non UTF-8 receipt".to_string()),
        project: Some("proj".to_string()),
        local_path: Some(local_path.display().to_string()),
        local_copy_enabled: Some(true),
        claim_enabled: Some(false),
        idempotency_key: Some("local-copy-non-utf8-receipt".to_string()),
        ..SaveMemoryRequest::default()
    };

    let error = save_memory(&conn, &req).expect_err("lossy receipt path must be rejected");

    assert!(error.to_string().contains("valid UTF-8"), "{error:#}");
    assert_eq!(
        std::fs::read(&invalid_target).expect("read restored target"),
        b"original"
    );
    let memory_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count memories");
    assert_eq!(memory_count, 0);
}
