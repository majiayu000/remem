use anyhow::{Context, Result};
use serde_json::json;

use crate::install::config::{build_hooks, build_mcp_server, remove_remem_hooks, remove_remem_mcp};
use crate::install::json_io::{read_json_file, write_json_file};
use crate::install::paths::{
    binary_path, claude_json_path, old_hooks_path, remem_data_dir, settings_path,
};

pub fn install() -> Result<()> {
    let bin = binary_path()?;
    let settings_file = settings_path();
    let claude_json_file = claude_json_path();

    let mut settings = read_json_file(&settings_file)?;
    remove_remem_hooks(&mut settings, &bin);
    remove_remem_mcp(&mut settings, &bin);

    let new_hooks = build_hooks(&bin);
    let obj = settings
        .as_object_mut()
        .context("settings.json 根节点不是 Object")?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if let (Some(existing), Some(new)) = (hooks.as_object_mut(), new_hooks.as_object()) {
        for (event_type, entries) in new {
            let arr = existing.entry(event_type).or_insert_with(|| json!([]));
            if let (Some(arr), Some(new_entries)) = (arr.as_array_mut(), entries.as_array()) {
                for entry in new_entries {
                    arr.push(entry.clone());
                }
            }
        }
    }
    write_json_file(&settings_file, &settings)?;

    let mut claude_json = read_json_file(&claude_json_file)?;
    remove_remem_mcp(&mut claude_json, &bin);

    let obj = claude_json
        .as_object_mut()
        .context("~/.claude.json 根节点不是 Object")?;
    let mcp_servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    if let Some(servers) = mcp_servers.as_object_mut() {
        servers.insert("remem".to_string(), build_mcp_server(&bin));
    }
    write_json_file(&claude_json_file, &claude_json)?;

    let data_dir = remem_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    eprintln!("remem install complete:");
    eprintln!("  hooks  -> {}", settings_file.display());
    eprintln!("  MCP    -> {}", claude_json_file.display());
    eprintln!("  data   -> {}", data_dir.display());
    eprintln!("  binary -> {}", bin);

    let old_path = old_hooks_path();
    if old_path.exists() {
        eprintln!();
        eprintln!("Legacy hooks.json detected: {}", old_path.display());
        eprintln!(
            "Claude Code does not read this file. Safe to delete: rm {}",
            old_path.display()
        );
    }

    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Restart Claude Code (quit and reopen)");
    eprintln!("  2. remem will automatically capture your sessions");
    eprintln!("  3. Run 'remem status' to check system health");

    Ok(())
}

pub fn uninstall() -> Result<()> {
    let bin = binary_path()?;
    let settings_file = settings_path();
    let claude_json_file = claude_json_path();

    if settings_file.exists() {
        let mut settings = read_json_file(&settings_file)?;
        remove_remem_hooks(&mut settings, &bin);
        remove_remem_mcp(&mut settings, &bin);
        write_json_file(&settings_file, &settings)?;
        eprintln!("  hooks 已从 {} 移除", settings_file.display());
    }

    if claude_json_file.exists() {
        let mut claude_json = read_json_file(&claude_json_file)?;
        remove_remem_mcp(&mut claude_json, &bin);
        write_json_file(&claude_json_file, &claude_json)?;
        eprintln!("  MCP 已从 {} 移除", claude_json_file.display());
    }

    eprintln!("remem uninstall 完成");
    eprintln!("  数据目录 {} 保留不动", remem_data_dir().display());

    Ok(())
}
