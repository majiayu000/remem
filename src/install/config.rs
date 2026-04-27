use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;

use crate::install::json_io::{read_json_file, write_json_file};

#[derive(Clone, Copy)]
pub(in crate::install) enum HookExecutor {
    ClaudeCli,
    CodexCli,
}

impl HookExecutor {
    fn summary_executor(self) -> &'static str {
        match self {
            Self::ClaudeCli => "claude-cli",
            Self::CodexCli => "codex-cli",
        }
    }
}

fn hook_command(bin: &str, executor: HookExecutor, subcommand: &str) -> String {
    if subcommand == "summarize" {
        format!(
            "REMEM_SUMMARY_EXECUTOR={} {} {}",
            executor.summary_executor(),
            bin,
            subcommand
        )
    } else {
        format!("{} {}", bin, subcommand)
    }
}

pub(in crate::install) fn build_hooks(bin: &str, executor: HookExecutor) -> Value {
    json!({
        "SessionStart": [{
            "hooks": [{ "type": "command", "command": hook_command(bin, executor, "context"), "timeout": 15000 }]
        }],
        "UserPromptSubmit": [{
            "hooks": [{ "type": "command", "command": hook_command(bin, executor, "session-init"), "timeout": 15000 }]
        }],
        "PostToolUse": [{
            "matcher": "Write|Edit|NotebookEdit|Bash|Task",
            "hooks": [{ "type": "command", "command": hook_command(bin, executor, "observe"), "timeout": 120000 }]
        }],
        "Stop": [{
            "hooks": [{ "type": "command", "command": hook_command(bin, executor, "summarize"), "timeout": 120000 }]
        }]
    })
}

pub(in crate::install) fn build_mcp_server(bin: &str) -> Value {
    json!({
        "type": "stdio",
        "command": bin,
        "args": ["mcp"]
    })
}

fn is_remem_hook(hook_entry: &Value, bin: &str) -> bool {
    if let Some(hooks) = hook_entry.get("hooks").and_then(|hooks| hooks.as_array()) {
        for hook in hooks {
            if let Some(cmd) = hook.get("command").and_then(|command| command.as_str()) {
                if cmd.contains(bin) || cmd.contains("remem") {
                    return true;
                }
            }
        }
    }
    false
}

pub(in crate::install) fn remove_remem_hooks(settings: &mut Value, bin: &str) {
    if let Some(hooks) = settings
        .get_mut("hooks")
        .and_then(|hooks| hooks.as_object_mut())
    {
        let event_types: Vec<String> = hooks.keys().cloned().collect();
        for event_type in event_types {
            if let Some(entries) = hooks
                .get_mut(&event_type)
                .and_then(|entries| entries.as_array_mut())
            {
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

/// Shared hook merge used by every host whose config file is JSON-shaped
/// the same way Claude Code's `settings.json` is.
///
/// Idempotent: strips any existing remem hook entries before appending fresh
/// ones, so repeated calls converge on the same state.
pub(in crate::install) fn apply_hooks_json(
    path: &Path,
    bin: &str,
    executor: HookExecutor,
) -> Result<()> {
    let mut doc = read_json_file(&path.to_path_buf())?;
    remove_remem_hooks(&mut doc, bin);

    let new_hooks = build_hooks(bin, executor);
    let obj = doc
        .as_object_mut()
        .with_context(|| format!("{} 根节点不是 Object", path.display()))?;
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
    write_json_file(&path.to_path_buf(), &doc)
}

/// Remove remem hook entries from the JSON file at `path`, if it exists.
pub(in crate::install) fn strip_hooks_json(path: &Path, bin: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut doc = read_json_file(&path.to_path_buf())?;
    remove_remem_hooks(&mut doc, bin);
    write_json_file(&path.to_path_buf(), &doc)
}

pub(in crate::install) fn remove_remem_mcp(settings: &mut Value, bin: &str) {
    if let Some(servers) = settings
        .get_mut("mcpServers")
        .and_then(|servers| servers.as_object_mut())
    {
        let keys: Vec<String> = servers.keys().cloned().collect();
        for key in keys {
            if key == "remem" {
                servers.remove(&key);
                continue;
            }
            if let Some(cmd) = servers
                .get(&key)
                .and_then(|server| server.get("command"))
                .and_then(|command| command.as_str())
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
