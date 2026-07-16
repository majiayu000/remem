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
    canonical_project_root_with_resolver(
        &canonical_cwd,
        git_environment_requires_resolver() || default_git_config_requires_resolver(),
        crate::git_util::resolve_toplevel,
    )
}

fn canonical_project_root_with_resolver(
    canonical_cwd: &std::path::Path,
    git_environment_requires_resolver: bool,
    mut resolve_toplevel: impl FnMut(&std::path::Path) -> Option<PathBuf>,
) -> PathBuf {
    if git_environment_requires_resolver {
        return resolve_toplevel(canonical_cwd)
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .unwrap_or_else(|| canonical_cwd.to_path_buf());
    }
    match git_worktree_root_from_markers(canonical_cwd) {
        GitMarkerDiscovery::Worktree(root) => root,
        GitMarkerDiscovery::RequiresResolver => resolve_toplevel(canonical_cwd)
            .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
            .unwrap_or_else(|| canonical_cwd.to_path_buf()),
        GitMarkerDiscovery::None => canonical_cwd.to_path_buf(),
    }
}

fn git_environment_requires_resolver() -> bool {
    git_environment_requires_resolver_with(|name| std::env::var_os(name).is_some())
}

fn git_environment_requires_resolver_with(mut is_set: impl FnMut(&str) -> bool) -> bool {
    [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_CONFIG",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ]
    .into_iter()
    .any(&mut is_set)
}

fn default_git_config_requires_resolver() -> bool {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let program_data = std::env::var_os("PROGRAMDATA").map(PathBuf::from);
    let system_paths = if cfg!(unix) && std::env::var_os("GIT_CONFIG_NOSYSTEM").is_none() {
        vec![PathBuf::from("/etc/gitconfig")]
    } else {
        Vec::new()
    };
    let paths = default_git_config_paths_with(
        home.as_deref(),
        xdg.as_deref(),
        program_data.as_deref(),
        system_paths,
    );
    git_config_paths_require_resolver(&paths)
}

fn default_git_config_paths_with(
    home: Option<&std::path::Path>,
    xdg: Option<&std::path::Path>,
    program_data: Option<&std::path::Path>,
    system_paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<PathBuf> {
    let mut paths = system_paths.into_iter().collect::<Vec<_>>();
    if let Some(program_data) = program_data {
        paths.push(program_data.join("Git/config"));
    }
    if let Some(xdg) = xdg.filter(|path| !path.as_os_str().is_empty()) {
        paths.push(xdg.join("git/config"));
    } else if let Some(home) = home {
        paths.push(home.join(".config/git/config"));
    }
    if let Some(home) = home {
        paths.push(home.join(".gitconfig"));
    }
    paths
}

fn git_config_paths_require_resolver(paths: &[PathBuf]) -> bool {
    paths
        .iter()
        .any(|path| match std::fs::read_to_string(path) {
            Ok(contents) => git_config_requires_resolver(&contents),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        })
}

#[derive(Debug, PartialEq, Eq)]
enum GitMarkerDiscovery {
    Worktree(PathBuf),
    RequiresResolver,
    None,
}

fn git_worktree_root_from_markers(cwd: &std::path::Path) -> GitMarkerDiscovery {
    for candidate in cwd.ancestors() {
        let marker = candidate.join(".git");
        match std::fs::symlink_metadata(&marker) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return GitMarkerDiscovery::RequiresResolver,
        }
        return if is_git_worktree_marker(&marker) {
            GitMarkerDiscovery::Worktree(candidate.to_path_buf())
        } else {
            GitMarkerDiscovery::RequiresResolver
        };
    }
    GitMarkerDiscovery::None
}

fn is_git_worktree_marker(marker: &std::path::Path) -> bool {
    if marker.is_dir() {
        return git_dir_has_plain_layout(marker) && !git_dir_config_requires_resolver(marker);
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
    git_dir_has_plain_layout(&git_dir) && !git_dir_config_requires_resolver(&git_dir)
}

fn git_dir_has_plain_layout(git_dir: &std::path::Path) -> bool {
    if !git_dir.join("HEAD").is_file() {
        return false;
    }
    let commondir = git_dir.join("commondir");
    let common_dir = if commondir.exists() {
        let Ok(common) = std::fs::read_to_string(&commondir) else {
            return false;
        };
        let common = common.trim();
        if common.is_empty() {
            return false;
        }
        let common = std::path::Path::new(common);
        if common.is_absolute() {
            common.to_path_buf()
        } else {
            git_dir.join(common)
        }
    } else {
        git_dir.to_path_buf()
    };

    common_dir.is_dir() && common_dir.join("objects").is_dir() && common_dir.join("refs").is_dir()
}

fn git_dir_config_requires_resolver(git_dir: &std::path::Path) -> bool {
    let mut config_dirs = vec![git_dir.to_path_buf()];
    let commondir = git_dir.join("commondir");
    if commondir.exists() {
        let Ok(common) = std::fs::read_to_string(&commondir) else {
            return true;
        };
        let common = common.trim();
        if common.is_empty() {
            return true;
        }
        let common = std::path::Path::new(common);
        let common = if common.is_absolute() {
            common.to_path_buf()
        } else {
            git_dir.join(common)
        };
        if !common.is_dir() {
            return true;
        }
        config_dirs.push(common);
    }
    config_dirs.into_iter().any(|dir| {
        let config = dir.join("config");
        match std::fs::read_to_string(config) {
            Ok(contents) => git_config_requires_resolver(&contents),
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    })
}

fn git_config_requires_resolver(contents: &str) -> bool {
    let mut section = "";
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            let Some(end) = line.find(']') else {
                return true;
            };
            section = line[1..end].split_whitespace().next().unwrap_or_default();
            if section.eq_ignore_ascii_case("include") || section.eq_ignore_ascii_case("includeif")
            {
                return true;
            }
            continue;
        }
        let mut fields = line.splitn(2, |ch: char| ch == '=' || ch.is_whitespace());
        let key = fields.next().unwrap_or_default().trim();
        let value = fields
            .next()
            .unwrap_or_default()
            .trim()
            .trim_start_matches('=')
            .trim();
        if section.eq_ignore_ascii_case("core") && key.eq_ignore_ascii_case("worktree") {
            return true;
        }
        if section.eq_ignore_ascii_case("core")
            && key.eq_ignore_ascii_case("bare")
            && !matches!(
                value.to_ascii_lowercase().as_str(),
                "false" | "no" | "off" | "0"
            )
        {
            return true;
        }
        if section.eq_ignore_ascii_case("extensions") && key.eq_ignore_ascii_case("worktreeconfig")
        {
            return true;
        }
    }
    false
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
        let common_dir = root.join("common-git-dir");
        let git_dir = common_dir.join("worktrees/linked");
        std::fs::create_dir_all(&nested)?;
        std::fs::create_dir_all(common_dir.join("objects"))?;
        std::fs::create_dir_all(common_dir.join("refs"))?;
        std::fs::create_dir_all(&git_dir)?;
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n")?;
        std::fs::write(git_dir.join("commondir"), "../..\n")?;
        std::fs::write(
            root.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )?;

        assert_eq!(
            git_worktree_root_from_markers(&nested),
            GitMarkerDiscovery::Worktree(root.clone()),
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

        assert_eq!(
            git_worktree_root_from_markers(&nested),
            GitMarkerDiscovery::RequiresResolver
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn incomplete_git_directory_delegates_to_git_and_fails_closed() -> anyhow::Result<()> {
        let root = unique_temp_path("incomplete-git-directory");
        let nested = root.join("nested");
        std::fs::create_dir_all(root.join(".git"))?;
        std::fs::create_dir_all(&nested)?;
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n")?;

        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(&nested)
            .output()?;
        assert!(
            !output.status.success(),
            "Git must reject the incomplete marker"
        );
        assert_eq!(
            git_worktree_root_from_markers(&nested),
            GitMarkerDiscovery::RequiresResolver,
            "an incomplete marker must not bypass Git's own validation"
        );
        let canonical_nested = nested.canonicalize()?;
        let mut resolver_called = false;
        let resolved = canonical_project_root_with_resolver(&canonical_nested, false, |_| {
            resolver_called = true;
            None
        });
        assert!(resolver_called, "incomplete markers must delegate to Git");
        assert_eq!(resolved, canonical_nested, "Git failure must fail closed");

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

    #[test]
    fn git_discovery_environment_requires_the_git_resolver() {
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_CEILING_DIRECTORIES",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_CONFIG",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
        ] {
            assert!(
                git_environment_requires_resolver_with(|candidate| candidate == variable),
                "{variable} must bypass marker discovery"
            );
        }
        assert!(!git_environment_requires_resolver_with(|_| false));
    }

    #[test]
    fn plain_git_config_keeps_the_marker_fast_path() {
        assert!(!git_config_requires_resolver(
            "[core]\n\trepositoryformatversion = 0\n\tbare = false\n"
        ));
        assert!(git_config_requires_resolver(
            "[core]\n\tworktree = ../configured\n"
        ));
    }

    #[test]
    fn default_global_xdg_and_system_configs_can_require_the_git_resolver() -> anyhow::Result<()> {
        let root = unique_temp_path("default-config-sources");
        let home = root.join("home");
        let xdg = root.join("xdg");
        let system = root.join("system.gitconfig");
        let global = home.join(".gitconfig");
        let xdg_config = xdg.join("git/config");
        std::fs::create_dir_all(xdg.join("git"))?;
        std::fs::create_dir_all(&home)?;
        for path in [&global, &xdg_config, &system] {
            std::fs::write(path, "[user]\n\tname = Test\n")?;
        }
        let paths = default_git_config_paths_with(
            Some(home.as_path()),
            Some(xdg.as_path()),
            None,
            [system.clone()],
        );

        assert!(!git_config_paths_require_resolver(&paths));
        for (source, path) in [
            ("global", &global),
            ("xdg", &xdg_config),
            ("system", &system),
        ] {
            std::fs::write(path, "[core]\n\tworktree = /tmp/configured\n")?;
            assert!(
                git_config_paths_require_resolver(&paths),
                "{source} core.worktree must require Git resolution"
            );
            std::fs::write(path, "[user]\n\tname = Test\n")?;
        }
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn core_worktree_config_precedes_the_marker_fast_path() -> anyhow::Result<()> {
        let root = unique_temp_path("core-worktree");
        let control = root.join("control");
        let configured_worktree = root.join("configured-worktree");
        let nested = control.join("nested");
        std::fs::create_dir_all(&nested)?;
        std::fs::create_dir_all(&configured_worktree)?;

        let status = Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(control.join(".git"))
            .status()?;
        assert!(
            status.success(),
            "bare control repository should initialize"
        );
        for args in [
            vec!["config", "core.bare", "false"],
            vec![
                "config",
                "core.worktree",
                configured_worktree.to_str().expect("utf-8 temp path"),
            ],
        ] {
            let status = Command::new("git")
                .arg(format!("--git-dir={}", control.join(".git").display()))
                .args(args)
                .status()?;
            assert!(status.success(), "git config should succeed");
        }

        assert_eq!(
            canonical_project_root(nested.to_str().expect("utf-8 temp path")),
            configured_worktree.canonicalize()?,
            "core.worktree must override the apparent .git marker parent"
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn malformed_nested_git_marker_stops_parent_discovery() -> anyhow::Result<()> {
        let root = unique_temp_path("malformed-nested-marker");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested)?;
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()?;
        assert!(status.success(), "parent repository should initialize");
        std::fs::write(nested.join(".git"), "not a gitdir marker\n")?;

        assert_eq!(
            canonical_project_root(nested.to_str().expect("utf-8 temp path")),
            nested.canonicalize()?,
            "Git rejects the malformed inner marker instead of discovering the parent repository"
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
