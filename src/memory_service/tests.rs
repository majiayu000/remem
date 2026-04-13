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

// --- TOCTOU / symlink-at-leaf tests ---

#[test]
#[cfg(unix)]
fn write_local_note_rejects_symlink_planted_at_leaf() {
    use std::os::unix::fs::symlink;
    let _dir = ScopedTestDataDir::new("toctou-leaf-symlink");
    let base = crate::db::data_dir();
    let parent = base.join("notes");
    std::fs::create_dir_all(&parent).unwrap();

    // Plant a symlink inside the data dir that points outside it.
    let target = base.join("notes").join("evil.md");
    symlink("/etc/passwd", &target).unwrap();

    let err = super::local_copy::write_local_note(&target, "should not write")
        .unwrap_err();
    assert!(
        err.to_string().contains("outside the allowed directory"),
        "expected confinement error, got: {}",
        err
    );
    // /etc/passwd must be untouched (we should never have written to it).
}

#[test]
#[cfg(unix)]
fn write_local_note_rejects_symlink_in_parent_dir() {
    use std::os::unix::fs::symlink;
    let _dir = ScopedTestDataDir::new("toctou-parent-symlink");
    let base = crate::db::data_dir();
    // ScopedTestDataDir sets the env var but does not create the directory.
    std::fs::create_dir_all(&base).unwrap();

    // Create a symlink at a directory component inside base pointing outside.
    // e.g. base/evil_dir -> /tmp  =>  base/evil_dir/file.md is outside base.
    let evil_dir = base.join("evil_dir");
    symlink("/tmp", &evil_dir).unwrap();
    let target = evil_dir.join("file.md");

    let err = super::local_copy::write_local_note(&target, "should not write")
        .unwrap_err();
    assert!(
        err.to_string().contains("outside the allowed directory"),
        "expected confinement error for symlinked parent, got: {}",
        err
    );
}
