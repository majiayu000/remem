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
    canonical_project_root_with_resolver(&canonical_cwd, explicit_git_layout(), |path| {
        crate::git_util::resolve_toplevel(path)
    })
}

fn canonical_project_root_with_resolver(
    canonical_cwd: &std::path::Path,
    explicit_git_layout: bool,
    mut resolve_toplevel: impl FnMut(&std::path::Path) -> Option<PathBuf>,
) -> PathBuf {
    if explicit_git_layout {
        return resolve_toplevel(canonical_cwd)
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .unwrap_or_else(|| canonical_cwd.to_path_buf());
    }
    if let Some(root) = git_worktree_root_from_markers(canonical_cwd) {
        return root;
    }
    let requires_git_fallback = canonical_cwd
        .ancestors()
        .any(|candidate| candidate.join(".git").exists());
    if requires_git_fallback {
        return resolve_toplevel(canonical_cwd)
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .unwrap_or_else(|| canonical_cwd.to_path_buf());
    }
    canonical_cwd.to_path_buf()
}

fn explicit_git_layout() -> bool {
    std::env::var_os("GIT_DIR").is_some() || std::env::var_os("GIT_WORK_TREE").is_some()
}

fn git_worktree_root_from_markers(cwd: &std::path::Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|candidate| is_git_worktree_marker(&candidate.join(".git")))
        .map(PathBuf::from)
}

fn is_git_worktree_marker(marker: &std::path::Path) -> bool {
    if marker.is_dir() {
        return marker.join("HEAD").is_file();
    }
    let Ok(contents) = std::fs::read_to_string(marker) else {
        return false;
    };
    let Some(git_dir) = contents
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return false;
    };
    let git_dir = std::path::Path::new(git_dir);
    let git_dir = if git_dir.is_absolute() {
        git_dir.to_path_buf()
    } else {
        marker
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(git_dir)
    };
    git_dir.join("HEAD").is_file()
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

    #[test]
    fn git_marker_file_identifies_a_linked_worktree_without_spawning_git() -> anyhow::Result<()> {
        let root = unique_temp_path("git-marker-file");
        let nested = root.join("crates").join("member").join("src");
        let git_dir = root.join("linked-worktree-git-dir");
        std::fs::create_dir_all(&nested)?;
        std::fs::create_dir_all(&git_dir)?;
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n")?;
        std::fs::write(root.join(".git"), "gitdir: linked-worktree-git-dir\n")?;

        assert_eq!(
            git_worktree_root_from_markers(&nested),
            Some(root.clone()),
            "a .git file is a worktree marker, not only .git directories"
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn invalid_git_marker_is_not_treated_as_a_worktree() -> anyhow::Result<()> {
        let root = unique_temp_path("invalid-git-marker");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested)?;
        std::fs::write(root.join(".git"), "not a gitdir marker\n")?;

        assert_eq!(git_worktree_root_from_markers(&nested), None);

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn explicit_git_layout_precedes_a_cwd_marker() -> anyhow::Result<()> {
        let cwd_root = unique_temp_path("explicit-layout-cwd");
        let selected_root = unique_temp_path("explicit-layout-selected");
        let nested = cwd_root.join("nested");
        std::fs::create_dir_all(cwd_root.join(".git"))?;
        std::fs::write(cwd_root.join(".git/HEAD"), "ref: refs/heads/main\n")?;
        std::fs::create_dir_all(&nested)?;
        std::fs::create_dir_all(&selected_root)?;

        let mut resolver_called = false;
        let resolved = canonical_project_root_with_resolver(&nested, true, |_| {
            resolver_called = true;
            Some(selected_root.clone())
        });

        assert!(resolver_called, "explicit layouts must consult Git");
        assert_eq!(resolved, selected_root.canonicalize()?);

        std::fs::remove_dir_all(cwd_root)?;
        std::fs::remove_dir_all(selected_root)?;
        Ok(())
    }
}
