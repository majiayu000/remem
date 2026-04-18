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
    let install_path = dirs::home_dir()
        .context("无法获取 HOME 目录")?
        .join(".local/bin/remem");
    install_path
        .to_str()
        .map(|s| s.to_string())
        .context("安装路径包含非 UTF-8 字符")
}
