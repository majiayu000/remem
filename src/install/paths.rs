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

pub(in crate::install) fn remem_data_dir() -> PathBuf {
    crate::db::data_dir()
}

pub(in crate::install) fn binary_path() -> Result<String> {
    let override_path = std::env::var("REMEM_INSTALL_BINARY").ok();
    let current_exe = std::env::current_exe().context("无法获取当前 remem 二进制路径")?;
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
        .context("remem 二进制路径包含非 UTF-8 字符")
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
