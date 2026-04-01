pub fn short_path(full: &str) -> &str {
    let parts: Vec<&str> = full.rsplitn(3, '/').collect();
    match parts.len() {
        1 => parts[0],
        2 => full,
        _ => {
            let start = full.len() - parts[0].len() - parts[1].len() - 1;
            &full[start..]
        }
    }
}

pub(super) fn extract_project_from_memory_path(file_path: &str) -> String {
    let Some(projects_pos) = file_path.find("/projects/") else {
        return "unknown".to_string();
    };
    let after_projects = &file_path[projects_pos + "/projects/".len()..];
    let slug = after_projects.split('/').next().unwrap_or("");
    if slug.is_empty() {
        return "unknown".to_string();
    }
    let mut decoded = slug.replace('-', "/");
    if !decoded.starts_with('/') {
        decoded = format!("/{decoded}");
    }
    crate::project_id::canonical_project_path(&decoded)
        .to_string_lossy()
        .to_string()
}
