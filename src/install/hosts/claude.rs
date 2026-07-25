use anyhow::{Context, Result};
use serde_json::json;
use std::path::PathBuf;

use crate::install::config::{
    apply_hooks_json, build_mcp_server, remove_remem_mcp, repair_hooks_json, strip_hooks_json,
    HookStrategy,
};
use crate::install::host::{HookRepairReport, HookSupport, InstallHost};
use crate::install::json_io::{read_json_file, write_json_file};
use crate::install::paths::{claude_json_path, claude_mcp_paths, settings_path};

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

    fn repair_hooks(&self, bin: &str) -> Result<HookRepairReport> {
        let path = settings_path();
        let integrity = repair_hooks_json(&path, bin, HookStrategy::ClaudeCode)?;
        let mcp_warning =
            match crate::hook_integrity::read_first_claude_mcp_command(&claude_mcp_paths()) {
                Ok(Some(command)) if command != bin => Some(format!(
                    "Claude MCP still points at {}; run `remem install --target claude` after repair if doctor reports install-path drift",
                    command
                )),
                Ok(_) => None,
                Err(error) => Some(format!("Could not inspect Claude MCP path: {error}")),
            };
        Ok(HookRepairReport {
            path,
            registered: integrity.registered,
            expected: integrity.expected,
            mcp_warning,
            scope_warning: Some(
                "Scope: repaired user-level ~/.claude/settings.json only; project/local/managed Claude hook locations were not modified"
                    .to_string(),
            ),
        })
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
                "  hooks  -> {} (SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/PreCompact/Stop)",
                settings_path().display()
            ),
            format!("  binary -> {}", bin),
        ]
    }
}
