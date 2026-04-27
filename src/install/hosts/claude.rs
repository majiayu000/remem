use anyhow::{Context, Result};
use serde_json::json;
use std::path::PathBuf;

use crate::install::config::{
    apply_hooks_json, build_mcp_server, remove_remem_mcp, strip_hooks_json, HookStrategy,
};
use crate::install::host::{HookSupport, InstallHost};
use crate::install::json_io::{read_json_file, write_json_file};
use crate::install::paths::{claude_json_path, settings_path};

pub(in crate::install) struct ClaudeHost;

impl InstallHost for ClaudeHost {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn config_path(&self) -> PathBuf {
        claude_json_path()
    }

    fn is_available(&self) -> bool {
        // Treat Claude as available if either of its config files exists.
        // (Fresh installs may have ~/.claude/ but no ~/.claude.json yet.)
        claude_json_path().exists()
            || dirs::home_dir()
                .map(|h| h.join(".claude").exists())
                .unwrap_or(false)
    }

    fn install_mcp(&self, bin: &str) -> Result<()> {
        let path = claude_json_path();
        let mut doc = read_json_file(&path)?;
        remove_remem_mcp(&mut doc, bin);
        let obj = doc
            .as_object_mut()
            .context("~/.claude.json 根节点不是 Object")?;
        let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
        if let Some(servers) = servers.as_object_mut() {
            servers.insert("remem".to_string(), build_mcp_server(bin));
        }
        write_json_file(&path, &doc)?;
        Ok(())
    }

    fn uninstall_mcp(&self, bin: &str) -> Result<()> {
        let path = claude_json_path();
        if !path.exists() {
            return Ok(());
        }
        let mut doc = read_json_file(&path)?;
        remove_remem_mcp(&mut doc, bin);
        write_json_file(&path, &doc)?;
        Ok(())
    }

    fn install_hooks(&self, bin: &str) -> Result<HookSupport> {
        // Claude's settings.json can historically end up with a stale
        // mcpServers entry if earlier versions mis-wrote here; clean that
        // up defensively before merging hooks.
        let path = settings_path();
        if path.exists() {
            let mut doc = read_json_file(&path)?;
            remove_remem_mcp(&mut doc, bin);
            write_json_file(&path, &doc)?;
        }
        apply_hooks_json(&path, bin, HookStrategy::ClaudeCode)?;
        Ok(HookSupport::Installed)
    }

    fn uninstall_hooks(&self, bin: &str) -> Result<()> {
        let path = settings_path();
        strip_hooks_json(&path, bin)?;
        if path.exists() {
            let mut doc = read_json_file(&path)?;
            remove_remem_mcp(&mut doc, bin);
            write_json_file(&path, &doc)?;
        }
        Ok(())
    }

    fn dry_run_plan(&self, bin: &str) -> Vec<String> {
        vec![
            format!(
                "  MCP    -> {} (add mcpServers.remem)",
                claude_json_path().display()
            ),
            format!(
                "  hooks  -> {} (SessionStart/UserPromptSubmit/PostToolUse/Stop)",
                settings_path().display()
            ),
            format!("  binary -> {}", bin),
        ]
    }
}
