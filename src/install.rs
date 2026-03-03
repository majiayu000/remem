use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Hooks 配置: ~/.claude/settings.json
fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}

/// MCP server 配置: ~/.claude.json (Claude Code 实际读取 MCP 的位置)
fn claude_json_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude.json")
}

fn old_hooks_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("hooks.json")
}

fn remem_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".remem")
}

fn binary_path() -> Result<String> {
    std::env::current_exe()
        .context("无法获取当前二进制路径")?
        .to_str()
        .map(|s| s.to_string())
        .context("二进制路径包含非 UTF-8 字符")
}

fn read_json_file(path: &PathBuf) -> Result<Value> {
    if path.exists() {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        serde_json::from_str(&content).with_context(|| format!("解析 {} 失败", path.display()))
    } else {
        Ok(json!({}))
    }
}

fn write_json_file(path: &PathBuf, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(value)?;
    std::fs::write(path, content).with_context(|| format!("写入 {} 失败", path.display()))
}

fn build_hooks(bin: &str) -> Value {
    json!({
        "SessionStart": [{
            "hooks": [{ "type": "command", "command": format!("{} context", bin), "timeout": 15000 }]
        }],
        "UserPromptSubmit": [{
            "hooks": [{ "type": "command", "command": format!("{} session-init", bin), "timeout": 15000 }]
        }],
        "PostToolUse": [{
            "matcher": "Write|Edit|NotebookEdit|Bash|Task",
            "hooks": [{ "type": "command", "command": format!("{} observe", bin), "timeout": 120000 }]
        }],
        "Stop": [{
            "hooks": [{ "type": "command", "command": format!("{} summarize", bin), "timeout": 120000 }]
        }]
    })
}

fn build_mcp_server(bin: &str) -> Value {
    json!({
        "type": "stdio",
        "command": bin,
        "args": ["mcp"]
    })
}

fn is_remem_hook(hook_entry: &Value, bin: &str) -> bool {
    if let Some(hooks) = hook_entry.get("hooks").and_then(|h| h.as_array()) {
        for h in hooks {
            if let Some(cmd) = h.get("command").and_then(|c| c.as_str()) {
                if cmd.contains(bin) || cmd.contains("remem") {
                    return true;
                }
            }
        }
    }
    false
}

fn remove_remem_hooks(settings: &mut Value, bin: &str) {
    if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        let event_types: Vec<String> = hooks.keys().cloned().collect();
        for event_type in event_types {
            if let Some(entries) = hooks.get_mut(&event_type).and_then(|v| v.as_array_mut()) {
                entries.retain(|entry| !is_remem_hook(entry, bin));
                if entries.is_empty() {
                    hooks.remove(&event_type);
                }
            }
        }
        if hooks.is_empty() {
            if let Some(obj) = settings.as_object_mut() {
                obj.remove("hooks");
            }
        }
    }
}

fn remove_remem_mcp(settings: &mut Value, bin: &str) {
    if let Some(servers) = settings
        .get_mut("mcpServers")
        .and_then(|s| s.as_object_mut())
    {
        let keys: Vec<String> = servers.keys().cloned().collect();
        for key in keys {
            if key == "remem" {
                servers.remove(&key);
                continue;
            }
            if let Some(cmd) = servers
                .get(&key)
                .and_then(|v| v.get("command"))
                .and_then(|c| c.as_str())
            {
                if cmd.contains(bin) || cmd.contains("remem") {
                    servers.remove(&key);
                }
            }
        }
        if servers.is_empty() {
            if let Some(obj) = settings.as_object_mut() {
                obj.remove("mcpServers");
            }
        }
    }
}

pub fn install() -> Result<()> {
    let bin = binary_path()?;
    let settings_file = settings_path();
    let claude_json_file = claude_json_path();

    // === 1. Hooks → ~/.claude/settings.json ===
    let mut settings = read_json_file(&settings_file)?;
    remove_remem_hooks(&mut settings, &bin);
    // 清理 settings.json 中残留的旧 MCP 配置
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

    // === 2. MCP server → ~/.claude.json ===
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

    // 创建数据目录
    let data_dir = remem_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    eprintln!("remem install 完成:");
    eprintln!("  hooks → {}", settings_file.display());
    eprintln!("  MCP   → {}", claude_json_file.display());
    eprintln!("  数据  → {}", data_dir.display());
    eprintln!("  二进制 → {}", bin);

    // 检查旧 hooks.json
    let old_path = old_hooks_path();
    if old_path.exists() {
        eprintln!();
        eprintln!("检测到旧版 hooks.json: {}", old_path.display());
        eprintln!(
            "Claude Code 不读取此文件，可以安全删除: rm {}",
            old_path.display()
        );
    }

    Ok(())
}

pub fn uninstall() -> Result<()> {
    let bin = binary_path()?;
    let settings_file = settings_path();
    let claude_json_file = claude_json_path();

    // 清理 hooks from settings.json
    if settings_file.exists() {
        let mut settings = read_json_file(&settings_file)?;
        remove_remem_hooks(&mut settings, &bin);
        remove_remem_mcp(&mut settings, &bin); // 清理残留
        write_json_file(&settings_file, &settings)?;
        eprintln!("  hooks 已从 {} 移除", settings_file.display());
    }

    // 清理 MCP from ~/.claude.json
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
