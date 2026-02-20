use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
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

fn read_settings() -> Result<Value> {
    let path = settings_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("解析 {} 失败", path.display()))
    } else {
        Ok(json!({}))
    }
}

fn write_settings(settings: &Value) -> Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(settings)?;
    std::fs::write(&path, content)
        .with_context(|| format!("写入 {} 失败", path.display()))
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
            "matcher": "Write|Edit|NotebookEdit|Bash",
            "hooks": [{ "type": "command", "command": format!("{} observe", bin), "timeout": 120000 }]
        }],
        "Stop": [{
            "hooks": [{ "type": "command", "command": format!("{} summarize", bin), "timeout": 120000 }]
        }]
    })
}

fn build_mcp_server(bin: &str) -> Value {
    json!({
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
    if let Some(servers) = settings.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        let keys: Vec<String> = servers.keys().cloned().collect();
        for key in keys {
            if key == "remem" {
                servers.remove(&key);
                continue;
            }
            if let Some(cmd) = servers.get(&key).and_then(|v| v.get("command")).and_then(|c| c.as_str()) {
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
    let mut settings = read_settings()?;

    // 清理旧的 remem 配置
    remove_remem_hooks(&mut settings, &bin);
    remove_remem_mcp(&mut settings, &bin);

    // 添加 hooks
    let new_hooks = build_hooks(&bin);
    let obj = settings.as_object_mut().context("settings.json 根节点不是 Object")?;
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if let (Some(existing), Some(new)) = (hooks.as_object_mut(), new_hooks.as_object()) {
        for (event_type, entries) in new {
            let arr = existing
                .entry(event_type)
                .or_insert_with(|| json!([]));
            if let (Some(arr), Some(new_entries)) = (arr.as_array_mut(), entries.as_array()) {
                for entry in new_entries {
                    arr.push(entry.clone());
                }
            }
        }
    }

    // 添加 MCP server
    let obj = settings.as_object_mut().context("settings.json 根节点不是 Object")?;
    let mcp_servers = obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if let Some(servers) = mcp_servers.as_object_mut() {
        servers.insert("remem".to_string(), build_mcp_server(&bin));
    }

    write_settings(&settings)?;

    // 创建数据目录
    let data_dir = remem_data_dir();
    std::fs::create_dir_all(&data_dir)?;

    eprintln!("remem install 完成:");
    eprintln!("  hooks + MCP → {}", settings_path().display());
    eprintln!("  数据目录    → {}", data_dir.display());
    eprintln!("  二进制路径  → {}", bin);

    // 检查旧 hooks.json
    let old_path = old_hooks_path();
    if old_path.exists() {
        eprintln!();
        eprintln!("检测到旧版 hooks.json: {}", old_path.display());
        eprintln!("Claude Code 不读取此文件，可以安全删除: rm {}", old_path.display());
    }

    Ok(())
}

pub fn uninstall() -> Result<()> {
    let bin = binary_path()?;
    let path = settings_path();
    if !path.exists() {
        eprintln!("settings.json 不存在，无需清理");
        return Ok(());
    }

    let mut settings = read_settings()?;
    remove_remem_hooks(&mut settings, &bin);
    remove_remem_mcp(&mut settings, &bin);
    write_settings(&settings)?;

    eprintln!("remem uninstall 完成:");
    eprintln!("  已从 {} 移除 hooks 和 MCP 配置", path.display());
    eprintln!("  数据目录 {} 保留不动", remem_data_dir().display());

    Ok(())
}
