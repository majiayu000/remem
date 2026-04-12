use super::{resolve_local_note_path, sanitize_segment};
use crate::db::test_support::ScopedTestDataDir;

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
