use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;

use crate::install::json_io::{read_json_file, write_json_file};

#[derive(Clone, Copy)]
pub(in crate::install) enum HookStrategy {
    ClaudeCode,
    Codex,
}

impl HookStrategy {
    fn include_session_init(self) -> bool {
        matches!(self, Self::ClaudeCode)
    }

    fn include_pre_compact(self) -> bool {
        matches!(self, Self::ClaudeCode)
    }

    fn session_start_matcher(self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some("startup|resume|clear|compact"),
            Self::Codex => None,
        }
    }

    fn context_timeout(self) -> i64 {
        match self {
            Self::ClaudeCode => 15,
            Self::Codex => 15000,
        }
    }

    fn observe_timeout(self) -> i64 {
        match self {
            Self::ClaudeCode => 120,
            Self::Codex => 120000,
        }
    }

    fn runtime_host(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex-cli",
        }
    }
}

fn hook_command(bin: &str, strategy: HookStrategy, subcommand: &str) -> String {
    format!(
        "{} {} --host {}",
        shell_quote(bin),
        subcommand,
        strategy.runtime_host()
    )
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-'))
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(in crate::install) fn build_hooks(bin: &str, strategy: HookStrategy) -> Value {
    let mut hooks = serde_json::Map::new();

    let mut session_start = json!({
        "hooks": [{ "type": "command", "command": hook_command(bin, strategy, "context"), "timeout": strategy.context_timeout() }]
    });
    if let Some(matcher) = strategy.session_start_matcher() {
        session_start["matcher"] = json!(matcher);
    }
    hooks.insert(
        "SessionStart".to_string(),
        Value::Array(vec![session_start]),
    );

    if strategy.include_session_init() {
        hooks.insert(
            "UserPromptSubmit".to_string(),
            json!([{
                "hooks": [{ "type": "command", "command": hook_command(bin, strategy, "session-init"), "timeout": strategy.context_timeout() }]
            }]),
        );
    }

    if matches!(strategy, HookStrategy::ClaudeCode) {
        hooks.insert(
            "PostToolUse".to_string(),
            json!([{
                "matcher": "Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task",
                "hooks": [{ "type": "command", "command": hook_command(bin, strategy, "observe"), "timeout": strategy.observe_timeout() }]
            }]),
        );
    }

    if strategy.include_pre_compact() {
        hooks.insert(
            "PreCompact".to_string(),
            json!([{
                "hooks": [{ "type": "command", "command": hook_command(bin, strategy, "summarize"), "timeout": strategy.observe_timeout() }]
            }]),
        );
    }

    hooks.insert(
        "Stop".to_string(),
        json!([{
            "hooks": [{ "type": "command", "command": hook_command(bin, strategy, "summarize"), "timeout": strategy.observe_timeout() }]
        }]),
    );

    Value::Object(hooks)
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
    strategy: HookStrategy,
) -> Result<()> {
    let mut doc = read_json_file(&path.to_path_buf())?;
    remove_remem_hooks(&mut doc, bin);

    let new_hooks = build_hooks(bin, strategy);
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

pub(in crate::install) fn repair_hooks_json(
    path: &Path,
    bin: &str,
    strategy: HookStrategy,
) -> Result<crate::hook_integrity::HookIntegrityReport> {
    let mut doc = read_json_file(&path.to_path_buf())?;
    let host = match strategy {
        HookStrategy::ClaudeCode => "claude",
        HookStrategy::Codex => "codex",
    };
    crate::hook_integrity::remove_remem_hooks_for_host(&mut doc, host);
    let new_hooks = build_hooks(bin, strategy);
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
    write_json_file(&path.to_path_buf(), &doc)?;
    let report =
        crate::hook_integrity::evaluate_hooks(&doc, host, path.to_path_buf(), Path::new(bin));
    if !report.is_healthy() {
        anyhow::bail!(
            "{} repair convergence failed: {}/{} registered; stale={:?}",
            path.display(),
            report.registered,
            report.expected,
            report.stale_details
        );
    }
    Ok(report)
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
