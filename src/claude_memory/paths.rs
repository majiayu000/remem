use std::path::PathBuf;

pub(super) fn encode_project_path(cwd: &str) -> String {
    let canonical = crate::db::canonical_project_path(cwd);
    let path_str = canonical.to_string_lossy();
    path_str.replace('/', "-")
}

pub(super) fn claude_memory_dir(cwd: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let encoded = encode_project_path(cwd);
    home.join(".claude")
        .join("projects")
        .join(&encoded)
        .join("memory")
}
