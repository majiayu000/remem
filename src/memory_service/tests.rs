use std::path::PathBuf;

use super::{resolve_local_note_path, sanitize_segment};

#[test]
fn resolve_local_note_path_makes_relative_paths_absolute() {
    let expected = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("docs/test.md");

    let got = resolve_local_note_path("manual", Some("x"), Some("docs/test.md"));

    assert_eq!(got, expected);
}

#[test]
fn sanitize_segment_falls_back_for_empty_slug() {
    let got = sanitize_segment("!!!", "fallback", 64);
    assert_eq!(got, "fallback");
}
