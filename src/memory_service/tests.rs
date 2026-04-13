use super::{resolve_local_note_path, sanitize_segment};
use crate::db::test_support::ScopedTestDataDir;

const LOCAL_SAVE_DIR_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_DIR";

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
    // PathBuf does not expand `~`; treated as literal dir name outside base
    let got = resolve_local_note_path("proj", Some("title"), Some("~/.ssh/authorized_keys"));
    assert!(got.is_err(), "tilde path should be rejected (not expanded)");
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
    let got = resolve_local_note_path("proj", Some("title"), None);
    assert!(got.is_ok());
    let path = got.unwrap();
    assert!(path.is_absolute());
    // Default path should be inside the data dir
    let base = crate::db::data_dir();
    assert!(
        path.starts_with(&base),
        "default path {:?} should be inside {:?}",
        path,
        base
    );
}

#[test]
fn resolve_default_path_with_env_outside_base_is_rejected() {
    // Regression test for SEC-07: REMEM_SAVE_MEMORY_LOCAL_DIR pointing outside
    // remem_data_dir() must be rejected even when local_path is None.
    let _dir = ScopedTestDataDir::new("path-env-outside");
    // Point LOCAL_SAVE_DIR_ENV to a directory outside the test base dir
    std::env::set_var(LOCAL_SAVE_DIR_ENV, "/tmp/evil-outside-base");
    let got = resolve_local_note_path("proj", Some("title"), None);
    std::env::remove_var(LOCAL_SAVE_DIR_ENV);
    assert!(
        got.is_err(),
        "default path via env outside base should be rejected, got {:?}",
        got
    );
    assert!(got
        .unwrap_err()
        .to_string()
        .contains("outside the allowed directory"));
}
