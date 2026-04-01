pub(super) fn resolve_cwd_project() -> (String, String) {
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let project = crate::db::project_from_cwd(&cwd);
    (cwd, project)
}
