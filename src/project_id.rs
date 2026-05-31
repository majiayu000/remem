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

/// Canonical project identity path.
///
/// Prefer the git worktree root when `cwd` is inside a repository so nested
/// directories in the same repo share one durable project. Fall back to the
/// canonical cwd for non-git directories and missing paths.
pub fn canonical_project_root(cwd: &str) -> PathBuf {
    let canonical_cwd = canonical_project_path(cwd);
    crate::git_util::resolve_toplevel(&canonical_cwd)
        .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
        .unwrap_or(canonical_cwd)
}

/// Canonical project identity (single source of truth).
pub fn project_from_cwd(cwd: &str) -> String {
    canonical_project_root(cwd).to_string_lossy().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remem-project-id-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn project_from_cwd_falls_back_to_canonical_cwd_outside_git() {
        let root = unique_temp_path("outside-git");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).expect("create temp dir");

        let expected = nested.canonicalize().expect("canonicalize temp dir");
        assert_eq!(
            project_from_cwd(nested.to_str().unwrap()),
            expected.display().to_string()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_from_cwd_prefers_git_toplevel_for_nested_cwd() {
        let root = unique_temp_path("git-root");
        let nested = root.join("crates").join("member").join("src");
        std::fs::create_dir_all(&nested).expect("create nested temp dir");
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .expect("spawn git init");
        assert!(status.success(), "git init should succeed");

        let expected = root.canonicalize().expect("canonicalize git root");
        assert_eq!(
            project_from_cwd(nested.to_str().unwrap()),
            expected.display().to_string()
        );
        assert_eq!(
            canonical_project_path(nested.to_str().unwrap()),
            nested.canonicalize().expect("canonicalize nested cwd"),
            "canonical cwd helper must remain a cwd path, not a project identity"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
