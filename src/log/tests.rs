use std::sync::Mutex;

use super::config::{log_max_bytes, log_path, DEFAULT_LOG_MAX_BYTES};
use super::open_log_append;
use super::write::rotate_if_needed;
use crate::db::test_support::ScopedTestDataDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_log_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().expect("log env lock should acquire");
    let previous = std::env::var("REMEM_LOG_MAX_BYTES").ok();

    match value {
        Some(value) => unsafe { std::env::set_var("REMEM_LOG_MAX_BYTES", value) },
        None => unsafe { std::env::remove_var("REMEM_LOG_MAX_BYTES") },
    }

    let result = f();

    match previous {
        Some(value) => unsafe { std::env::set_var("REMEM_LOG_MAX_BYTES", value) },
        None => unsafe { std::env::remove_var("REMEM_LOG_MAX_BYTES") },
    }

    result
}

#[test]
fn log_max_bytes_uses_positive_env_override() {
    with_log_env(Some("4096"), || {
        assert_eq!(log_max_bytes(), 4096);
    });
}

#[test]
fn log_max_bytes_rejects_zero_and_invalid() {
    with_log_env(Some("0"), || {
        assert_eq!(log_max_bytes(), DEFAULT_LOG_MAX_BYTES);
    });
    with_log_env(Some("invalid"), || {
        assert_eq!(log_max_bytes(), DEFAULT_LOG_MAX_BYTES);
    });
}

#[test]
fn open_log_append_creates_log_file_in_data_dir() {
    let _data_dir = ScopedTestDataDir::new("log-open-append");

    let file = open_log_append().expect("log file should open");
    drop(file);

    let path = log_path().expect("log path should resolve");
    assert!(path.exists(), "log file should exist at {:?}", path);
}

#[test]
fn rotate_if_needed_shifts_existing_files() {
    let _data_dir = ScopedTestDataDir::new("log-rotate");
    let path = log_path().expect("log path should resolve");
    let parent = path.parent().expect("log file should have parent");
    std::fs::create_dir_all(parent).expect("log dir should create");

    std::fs::write(&path, "base-payload").expect("base log should write");
    std::fs::write(format!("{}.1", path.display()), "older-1").expect("log.1 should write");
    std::fs::write(format!("{}.2", path.display()), "older-2").expect("log.2 should write");
    std::fs::write(format!("{}.3", path.display()), "older-3").expect("log.3 should write");

    rotate_if_needed(&path, 4);

    assert!(
        !path.exists(),
        "base log should be renamed away during rotation"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.1", path.display())).expect("log.1 should read"),
        "base-payload"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.2", path.display())).expect("log.2 should read"),
        "older-1"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{}.3", path.display())).expect("log.3 should read"),
        "older-2"
    );
}
