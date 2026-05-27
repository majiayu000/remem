//! Minimal git CLI helpers used by capture identity. The pure parser is
//! split from the side-effecting subprocess spawn so the parser stays
//! unit-testable; the spawn itself is documented but not unit-tested (it
//! would require tempdir + git init in the test environment, which adds an
//! external dependency for a single helper).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitMetadata {
    pub repo_path: String,
    pub sha: String,
    pub short_sha: String,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub authored_at_epoch: Option<i64>,
    pub changed_files: Vec<String>,
}

impl GitCommitMetadata {
    pub fn matches_sha(&self, sha: &str) -> bool {
        let needle = sha.trim();
        !needle.is_empty()
            && (self.sha == needle
                || self.short_sha == needle
                || self.sha.starts_with(needle)
                || self.short_sha.starts_with(needle))
    }
}

/// Parse the output of `git rev-parse --show-toplevel`. Returns `None` for
/// empty or whitespace-only input so callers can fall back cleanly.
pub fn parse_toplevel_output(stdout: &str) -> Option<PathBuf> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Spawn `git rev-parse --show-toplevel` in `cwd`. Returns `None` when git
/// is unavailable, the directory is outside any git worktree, or the output
/// is empty. Single subprocess, no network.
pub fn resolve_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_toplevel_output(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_changed_files_output(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn parse_authored_epoch_output(stdout: &str) -> Option<i64> {
    stdout.trim().parse::<i64>().ok()
}

pub fn parse_branch_output(stdout: &str) -> Option<String> {
    let branch = stdout.trim();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch.to_string())
    }
}

pub fn short_sha_for(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub fn detect_commit_metadata(cwd: &str) -> Option<GitCommitMetadata> {
    let repo_path = resolve_toplevel(Path::new(cwd))?;
    let sha = git_stdout(cwd, &["rev-parse", "HEAD"])?;
    let sha = sha.trim();
    if sha.is_empty() {
        return None;
    }

    let short_sha = git_stdout(cwd, &["rev-parse", "--short", "HEAD"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| short_sha_for(sha));
    let branch = git_stdout(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .and_then(|value| parse_branch_output(&value));
    let message = git_stdout(cwd, &["show", "-s", "--format=%s", "HEAD"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let authored_at_epoch = git_stdout(cwd, &["show", "-s", "--format=%ct", "HEAD"])
        .and_then(|value| parse_authored_epoch_output(&value));
    let changed_files = git_stdout(
        cwd,
        &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"],
    )
    .map(|value| parse_changed_files_output(&value))
    .unwrap_or_default();

    Some(GitCommitMetadata {
        repo_path: repo_path.to_string_lossy().to_string(),
        sha: sha.to_string(),
        short_sha,
        branch,
        message,
        authored_at_epoch,
        changed_files,
    })
}

fn git_stdout(cwd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_returns_path_for_normal_output() {
        assert_eq!(
            parse_toplevel_output("/Users/foo/project\n"),
            Some(PathBuf::from("/Users/foo/project"))
        );
    }

    #[test]
    fn parse_returns_none_for_empty_or_whitespace() {
        assert_eq!(parse_toplevel_output(""), None);
        assert_eq!(parse_toplevel_output("\n"), None);
        assert_eq!(parse_toplevel_output("   \t\n  "), None);
    }

    #[test]
    fn parse_strips_trailing_whitespace() {
        assert_eq!(
            parse_toplevel_output("/path/with-spaces  \n\t"),
            Some(PathBuf::from("/path/with-spaces"))
        );
    }

    #[test]
    fn parse_changed_files_ignores_blank_lines() {
        assert_eq!(
            parse_changed_files_output("src/lib.rs\n\n  README.md  \n"),
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
    }

    #[test]
    fn parse_authored_epoch_requires_integer() {
        assert_eq!(
            parse_authored_epoch_output("1700000000\n"),
            Some(1700000000)
        );
        assert_eq!(parse_authored_epoch_output("not-an-epoch"), None);
    }

    #[test]
    fn parse_branch_ignores_detached_head() {
        assert_eq!(
            parse_branch_output("feature/x\n"),
            Some("feature/x".to_string())
        );
        assert_eq!(parse_branch_output("HEAD\n"), None);
        assert_eq!(parse_branch_output("\n"), None);
    }

    #[test]
    fn commit_metadata_matches_full_short_and_prefix() {
        let metadata = GitCommitMetadata {
            repo_path: "/repo".to_string(),
            sha: "abcdef1234567890".to_string(),
            short_sha: "abcdef1".to_string(),
            branch: Some("main".to_string()),
            message: None,
            authored_at_epoch: None,
            changed_files: Vec::new(),
        };

        assert!(metadata.matches_sha("abcdef1234567890"));
        assert!(metadata.matches_sha("abcdef1"));
        assert!(metadata.matches_sha("abcdef"));
        assert!(!metadata.matches_sha(""));
        assert!(!metadata.matches_sha("fedcba"));
    }
}
