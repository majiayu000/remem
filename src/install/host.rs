use anyhow::Result;
use clap::ValueEnum;
use std::path::PathBuf;

/// Which host(s) to (un)install into.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum InstallTarget {
    /// Install to every host whose config directory already exists.
    Auto,
    /// Install only to Claude Code (~/.claude.json + ~/.claude/settings.json).
    Claude,
    /// Install only to Codex (~/.codex/config.toml).
    Codex,
    /// Install to every known host, creating config files if missing.
    All,
}

/// Outcome of a hook installation attempt.
pub enum HookSupport {
    /// Hooks were installed.
    Installed,
    /// Host does not support hooks; reason for user-visible log.
    /// Retained for future hosts (e.g. Cursor) that may not expose a hook
    /// system with the same shape as Claude/Codex.
    #[allow(dead_code)]
    Skipped(&'static str),
}

/// A host capable of running the remem MCP server.
///
/// Each host owns its own config file format and mutation logic. The runtime
/// layer only orchestrates which hosts to touch.
pub trait InstallHost {
    /// Short identifier printed in logs (e.g. "claude", "codex").
    fn name(&self) -> &'static str;

    /// Path to the primary config file this host manages.
    fn config_path(&self) -> PathBuf;

    /// True when the host appears to be installed on this machine.
    /// Used by `InstallTarget::Auto` to decide whether to touch it.
    fn is_available(&self) -> bool;

    /// Add / update the remem MCP server entry. Idempotent.
    fn install_mcp(&self, bin: &str) -> Result<()>;

    /// Remove any remem MCP server entry. Idempotent.
    fn uninstall_mcp(&self, bin: &str) -> Result<()>;

    /// Add / update remem hooks. Hosts without hook support return `Skipped`.
    fn install_hooks(&self, bin: &str) -> Result<HookSupport>;

    /// Remove remem hooks. No-op if the host doesn't support hooks.
    fn uninstall_hooks(&self, bin: &str) -> Result<()>;

    /// Describe the writes a real install would do, without touching disk.
    /// Returned lines are printed verbatim in dry-run mode.
    fn dry_run_plan(&self, bin: &str) -> Vec<String>;
}
