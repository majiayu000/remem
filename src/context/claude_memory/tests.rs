use std::path::PathBuf;

use super::index::ensure_memory_index;
use super::paths::encode_project_path;

fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("remem-{}-{}-{}", name, std::process::id(), nanos))
}

#[test]
fn encode_project_path_replaces_slashes_after_canonicalize() {
    let dir = unique_temp_dir("claude-memory-path");
    std::fs::create_dir_all(&dir).expect("temp dir should create");

    let encoded = encode_project_path(dir.to_str().expect("temp path should be utf-8"));
    let canonical =
        crate::db::canonical_project_path(dir.to_str().expect("temp path should be utf-8"));
    let expected = canonical.to_string_lossy().replace('/', "-");

    assert_eq!(encoded, expected);

    std::fs::remove_dir_all(&dir).expect("temp dir should clean up");
}

#[test]
fn ensure_memory_index_is_idempotent() {
    let dir = unique_temp_dir("claude-memory-index-idempotent");
    std::fs::create_dir_all(&dir).expect("temp dir should create");
    let index_path = dir.join("MEMORY.md");
    std::fs::write(&index_path, "# Memory Index\n").expect("index should write");

    ensure_memory_index(&dir).expect("first ensure should succeed");
    ensure_memory_index(&dir).expect("second ensure should succeed");

    let content = std::fs::read_to_string(&index_path).expect("index should read");
    assert_eq!(content.matches("remem_sessions.md").count(), 1);

    std::fs::remove_dir_all(&dir).expect("temp dir should clean up");
}

#[test]
fn ensure_memory_index_creates_new_file_when_missing() {
    let dir = unique_temp_dir("claude-memory-index-create");
    std::fs::create_dir_all(&dir).expect("temp dir should create");

    ensure_memory_index(&dir).expect("ensure should succeed");

    let content = std::fs::read_to_string(dir.join("MEMORY.md")).expect("index should read");
    assert!(content.contains("# Memory Index"));
    assert!(content.contains("remem_sessions.md"));

    std::fs::remove_dir_all(&dir).expect("temp dir should clean up");
}
