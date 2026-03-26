use std::path::PathBuf;

/// Build canonical absolute path for cwd-like inputs.
pub fn canonical_project_path(cwd: &str) -> PathBuf {
    let path = std::path::Path::new(cwd);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    std::fs::canonicalize(&abs).unwrap_or_else(|e| {
        crate::log::warn(
            "project-id",
            &format!("canonicalize {:?} failed (using abs): {}", abs, e),
        );
        abs
    })
}

/// Canonical project identity (single source of truth).
/// This intentionally uses full canonical path to avoid cross-repo key collisions.
pub fn project_from_cwd(cwd: &str) -> String {
    canonical_project_path(cwd).to_string_lossy().to_string()
}

/// Push exact project filter SQL and parameter.
pub fn push_project_filter(
    column: &str,
    project: &str,
    idx: usize,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> (String, usize) {
    let clause = format!("{column} = ?{idx}");
    params.push(Box::new(project.to_string()));
    (clause, idx + 1)
}

pub fn project_matches(value: Option<&str>, project: &str) -> bool {
    value.is_some_and(|v| v == project)
}
