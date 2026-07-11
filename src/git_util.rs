//! Minimal git CLI helpers used by capture identity. The pure parser is
//! split from the side-effecting subprocess spawn so the parser stays
//! unit-testable; the spawn itself is documented but not unit-tested (it
//! would require tempdir + git init in the test environment, which adds an
//! external dependency for a single helper).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

const COMMIT_METADATA_FORMAT: &str = "--format=%H%x00%h%x00%at%x00%s";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitEvidenceKind {
    ObservedCommit,
    TerminalSnapshot,
}

impl GitEvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObservedCommit => "observed_commit",
            Self::TerminalSnapshot => "terminal_snapshot",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitCommitMetadata {
    pub repo_path: String,
    pub sha: String,
    pub short_sha: String,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub authored_at_epoch: Option<i64>,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitCommitEvidence {
    pub kind: GitEvidenceKind,
    pub metadata: GitCommitMetadata,
    pub locator: Option<String>,
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
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for file in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if seen.insert(file.to_string()) {
            files.push(file.to_string());
        }
    }
    files
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

pub fn detect_commit_metadata(cwd: &str) -> Result<Option<GitCommitMetadata>> {
    let Some(_) = resolve_toplevel(Path::new(cwd)) else {
        return Ok(None);
    };
    let sha = git_stdout_required(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let sha = sha.trim().to_ascii_lowercase();
    if sha.is_empty() {
        bail!("git returned an empty HEAD commit in {cwd}");
    }

    let metadata = resolve_commit_metadata(cwd, &sha)?;
    let final_sha = git_stdout_required(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !final_sha.trim().eq_ignore_ascii_case(&sha) {
        bail!("git HEAD changed while capturing commit metadata in {cwd}");
    }
    Ok(Some(metadata))
}

pub fn resolve_commit_metadata(cwd: &str, commitish: &str) -> Result<GitCommitMetadata> {
    let repo_path = resolve_toplevel(Path::new(cwd))
        .with_context(|| format!("resolve Git repository for commit evidence in {cwd}"))?;
    let commit_ref = format!("{}^{{commit}}", commitish.trim());
    let sha = git_stdout_required(cwd, &["rev-parse", "--verify", &commit_ref])?;
    let sha = sha.trim().to_ascii_lowercase();
    if sha.is_empty() {
        bail!("git returned an empty commit for evidence {commitish}");
    }
    let raw_metadata = git_stdout_required(cwd, &["show", "-s", COMMIT_METADATA_FORMAT, &sha])?;
    let mut fields = raw_metadata.trim_end_matches(['\r', '\n']).splitn(4, '\0');
    let resolved_sha = fields
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let short_sha = fields
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let authored_at_epoch = fields
        .next()
        .and_then(parse_authored_epoch_output)
        .context("git commit metadata omitted authored epoch")?;
    let message = fields
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if resolved_sha != sha || short_sha.is_empty() || !sha.starts_with(&short_sha) {
        bail!("git returned inconsistent metadata for captured commit {sha}");
    }

    let current_head = git_stdout_required(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let branch = if current_head.trim().eq_ignore_ascii_case(&sha) {
        parse_branch_output(&git_stdout_required(cwd, &["branch", "--show-current"])?)
    } else {
        None
    };
    let changed_files = parse_changed_files_output(&git_stdout_required(
        cwd,
        &[
            "diff-tree",
            "--root",
            "-m",
            "--no-commit-id",
            "--name-only",
            "-r",
            &sha,
        ],
    )?);
    Ok(sanitize_commit_metadata(GitCommitMetadata {
        repo_path: repo_path.to_string_lossy().to_string(),
        sha,
        short_sha,
        branch,
        message,
        authored_at_epoch: Some(authored_at_epoch),
        changed_files,
    }))
}

pub fn sanitize_commit_metadata(mut metadata: GitCommitMetadata) -> GitCommitMetadata {
    metadata.branch = metadata
        .branch
        .map(|value| crate::adapter::common::redact_sensitive_text(&value));
    metadata.message = metadata
        .message
        .map(|value| crate::adapter::common::redact_sensitive_text(&value));
    metadata.changed_files = metadata
        .changed_files
        .into_iter()
        .map(|value| crate::adapter::common::redact_sensitive_text(&value))
        .collect();
    metadata
}

fn git_stdout_required(cwd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .with_context(|| format!("run git {} in {cwd}", args.join(" ")))?;
    if !output.status.success() {
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        let stderr = crate::db::truncate_str(&stderr_text, 400);
        bail!(
            "git {} failed in {cwd} with status {}: {stderr}",
            args.join(" "),
            output.status
        );
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("git {} returned non-UTF-8 output", args.join(" ")))
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
    fn parse_changed_files_deduplicates_merge_diff_output() {
        assert_eq!(
            parse_changed_files_output("src/lib.rs\nREADME.md\nsrc/lib.rs\nREADME.md\n"),
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
    }

    #[test]
    fn metadata_detection_uses_one_anchored_metadata_format() {
        assert!(COMMIT_METADATA_FORMAT.contains("%H"));
        assert!(COMMIT_METADATA_FORMAT.contains("%h"));
        assert!(COMMIT_METADATA_FORMAT.contains("%at"));
        assert!(COMMIT_METADATA_FORMAT.contains("%s"));
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
