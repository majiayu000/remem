use serde_json::{json, Value};

pub(super) fn build_hooks(bin: &str) -> Value {
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

pub(super) fn build_mcp_server(bin: &str) -> Value {
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

pub(super) fn remove_remem_hooks(settings: &mut Value, bin: &str) {
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

pub(super) fn remove_remem_mcp(settings: &mut Value, bin: &str) {
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
