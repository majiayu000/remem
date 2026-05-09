//! Minimal git CLI helpers used by v2 capture identity. The pure parser is
//! split from the side-effecting subprocess spawn so the parser stays
//! unit-testable; the spawn itself is documented but not unit-tested (it
//! would require tempdir + git init in the test environment, which adds an
//! external dependency for a single helper).

use std::path::{Path, PathBuf};
use std::process::Command;

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
}
