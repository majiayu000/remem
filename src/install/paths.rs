use anyhow::{Context, Result};
use std::path::PathBuf;

pub(in crate::install) fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}

pub(in crate::install) fn claude_json_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude.json")
}

pub(in crate::install) fn claude_desktop_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("claude_desktop_config.json")
}

pub(crate) fn claude_mcp_paths() -> Vec<PathBuf> {
    vec![claude_json_path(), claude_desktop_config_path()]
}

pub(in crate::install) fn old_hooks_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("hooks.json")
}

pub(in crate::install) fn codex_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("config.toml")
}

pub(in crate::install) fn codex_hooks_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("hooks.json")
}

pub(in crate::install) fn cursor_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cursor")
}

/// User-level Cursor hooks config (`~/.cursor/hooks.json`). GH-824 never
/// touches project-level `<project>/.cursor/hooks.json`.
pub(in crate::install) fn cursor_hooks_path() -> PathBuf {
    cursor_dir().join("hooks.json")
}

/// User-level Cursor MCP config (`~/.cursor/mcp.json`).
pub(in crate::install) fn cursor_mcp_path() -> PathBuf {
    cursor_dir().join("mcp.json")
}

/// Official Codex CLI user-level rollout-summary memory location, verified on
/// codex-cli 0.145.0 (docs/research/gh852-host-native-memory-poc.md).
pub(crate) fn codex_memories_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("memories")
        .join("rollout_summaries")
}

/// Filename fingerprint of codex-rollout-summary/v1, verified against a real
/// codex-cli 0.145.0 installation (95/95 files matched):
/// `YYYY-MM-DDTHH-MM-SS-<4 alnum>-<slug>.md`. Shared by the import discovery
/// walk and the doctor source-state check (GH-852).
pub(crate) fn is_codex_rollout_summary_filename(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".md") else {
        return false;
    };
    let bytes = stem.as_bytes();
    // 2026-05-27T09-52-53-bZ3O- prefix = 25 bytes, then a non-empty slug.
    if bytes.len() < 26 {
        return false;
    }
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    if !digits.iter().all(|&idx| bytes[idx].is_ascii_digit()) {
        return false;
    }
    let separators = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b'-'),
        (16, b'-'),
        (19, b'-'),
        (24, b'-'),
    ];
    if !separators.iter().all(|&(idx, ch)| bytes[idx] == ch) {
        return false;
    }
    if !bytes[20..24].iter().all(u8::is_ascii_alphanumeric) {
        return false;
    }
    bytes[25..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

pub(in crate::install) fn remem_data_dir() -> Result<PathBuf> {
    crate::db::try_data_dir()
}

pub(in crate::install) fn binary_path() -> Result<String> {
    let override_path = std::env::var("REMEM_INSTALL_BINARY").ok();
    let current_exe =
        std::env::current_exe().context("failed to resolve the current remem binary path")?;
    resolve_binary_path(override_path, current_exe)
}

fn resolve_binary_path(override_path: Option<String>, current_exe: PathBuf) -> Result<String> {
    if let Some(path) = override_path {
        if !path.trim().is_empty() {
            return Ok(path);
        }
    }

    current_exe
        .to_str()
        .map(|s| s.to_string())
        .context("remem binary path contains non-UTF-8 characters")
}

#[cfg(test)]
mod tests {
    use super::resolve_binary_path;
    use std::path::PathBuf;

    #[test]
    fn binary_path_override_wins() {
        let result = resolve_binary_path(
            Some("/custom/bin/remem".to_string()),
            PathBuf::from("/current/bin/remem"),
        );
        let Ok(path) = result else {
            panic!("override path should be valid");
        };
        assert_eq!(path, "/custom/bin/remem");
    }

    #[test]
    fn binary_path_uses_current_exe_without_override() {
        let result = resolve_binary_path(None, PathBuf::from("/current/bin/remem"));
        let Ok(path) = result else {
            panic!("current exe path should be valid");
        };
        assert_eq!(path, "/current/bin/remem");
    }

    #[test]
    fn binary_path_ignores_blank_override() {
        let result =
            resolve_binary_path(Some("  ".to_string()), PathBuf::from("/current/bin/remem"));
        let Ok(path) = result else {
            panic!("current exe path should be valid when override is blank");
        };
        assert_eq!(path, "/current/bin/remem");
    }
}
