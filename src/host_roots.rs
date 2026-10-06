//! Shared host-profile selection for native readers, installers and diagnostics.

use anyhow::{ensure, Context, Result};
use std::path::PathBuf;

/// Keep the default host home consistent with agent-sessions on Windows,
/// including explicit process-local profile selection.
pub(crate) fn home_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        windows_home_override(
            std::env::var_os("HOME").map(PathBuf::from),
            std::env::var_os("USERPROFILE").map(PathBuf::from),
        )
        .or_else(dirs::home_dir)
    } else {
        dirs::home_dir()
    }
}

fn windows_home_override(home: Option<PathBuf>, user_profile: Option<PathBuf>) -> Option<PathBuf> {
    home.into_iter()
        .chain(user_profile)
        .find(|path| path.is_absolute())
}

pub(crate) fn codex() -> Result<PathBuf> {
    let root = agent_sessions::Roots::from_env_for(agent_sessions::Agent::Codex)?
        .codex
        .context("cannot resolve Codex home: set an absolute CODEX_HOME")?;
    validate_codex_root(root)
}

fn validate_codex_root(root: PathBuf) -> Result<PathBuf> {
    ensure!(
        root.is_absolute(),
        "Codex home must be absolute; set CODEX_HOME to an absolute directory"
    );
    match std::fs::metadata(&root) {
        Ok(metadata) => ensure!(metadata.is_dir(), "Codex home is not a directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("cannot inspect Codex home"),
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute_home(name: &str) -> PathBuf {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\fixture-profiles")
        } else {
            PathBuf::from("/fixture-profiles")
        };
        root.join(name)
    }

    #[test]
    fn windows_home_override_prefers_absolute_home() {
        let home = absolute_home("home");
        let profile = absolute_home("profile");
        assert_eq!(
            windows_home_override(Some(home.clone()), Some(profile)),
            Some(home)
        );
    }

    #[test]
    fn windows_home_override_ignores_empty_paths() {
        let profile = absolute_home("profile");
        assert_eq!(
            windows_home_override(Some(PathBuf::new()), Some(profile.clone())),
            Some(profile)
        );
        assert_eq!(
            windows_home_override(Some(PathBuf::new()), Some(PathBuf::new())),
            None
        );
    }

    #[test]
    fn windows_home_override_ignores_relative_paths() {
        let profile = absolute_home("profile");
        assert_eq!(
            windows_home_override(Some(PathBuf::from("relative-home")), Some(profile.clone())),
            Some(profile)
        );
        assert_eq!(
            windows_home_override(
                Some(PathBuf::from("relative-home")),
                Some(PathBuf::from("relative-profile"))
            ),
            None
        );
    }

    #[test]
    fn windows_home_override_without_home_uses_user_profile_or_none() {
        assert_eq!(windows_home_override(None, None), None);
        let profile = absolute_home("profile");
        assert_eq!(
            windows_home_override(None, Some(profile.clone())),
            Some(profile)
        );
    }

    #[test]
    fn codex_root_requires_an_absolute_path() {
        for value in ["", " ", "profile", "./profile", "~/profile"] {
            assert!(validate_codex_root(PathBuf::from(value)).is_err());
        }
        let root = std::env::temp_dir().join("remem-codex-root");
        assert_eq!(validate_codex_root(root.clone()).unwrap(), root);
    }
}
