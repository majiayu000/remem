use serde_json::json;

use super::config::{build_hooks, remove_remem_hooks, remove_remem_mcp, HookStrategy};

#[test]
fn build_hooks_contains_expected_claude_commands() {
    let hooks = build_hooks("/tmp/remem", HookStrategy::ClaudeCode);
    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "/tmp/remem context --host claude-code"
    );
    assert_eq!(
        hooks["UserPromptSubmit"][0]["hooks"][0]["command"],
        "/tmp/remem session-init --host claude-code"
    );
    assert_eq!(
        hooks["PostToolUse"][0]["hooks"][0]["command"],
        "/tmp/remem observe --host claude-code"
    );
    assert_eq!(
        hooks["PostToolUse"][0]["matcher"],
        "Write|Edit|NotebookEdit|Bash|Grep|Glob|Task"
    );
    assert_eq!(
        hooks["Stop"][0]["hooks"][0]["command"],
        "/tmp/remem summarize --host claude-code"
    );
}

#[test]
fn build_hooks_contains_expected_codex_commands() {
    let hooks = build_hooks("/tmp/remem", HookStrategy::Codex);
    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "/tmp/remem context --host codex-cli"
    );
    assert!(hooks.get("UserPromptSubmit").is_none());
    assert!(hooks.get("PostToolUse").is_none());
    assert_eq!(
        hooks["Stop"][0]["hooks"][0]["command"],
        "/tmp/remem summarize --host codex-cli"
    );
}

#[test]
fn build_hooks_quotes_binary_paths_with_spaces() {
    let hooks = build_hooks("/tmp/remem bin/remem", HookStrategy::Codex);

    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "'/tmp/remem bin/remem' context --host codex-cli"
    );
    assert_eq!(
        hooks["Stop"][0]["hooks"][0]["command"],
        "'/tmp/remem bin/remem' summarize --host codex-cli"
    );
}

#[test]
fn build_hooks_quotes_binary_paths_with_single_quotes() {
    let hooks = build_hooks("/tmp/remem'bin/remem", HookStrategy::ClaudeCode);

    assert_eq!(
        hooks["PostToolUse"][0]["hooks"][0]["command"],
        "'/tmp/remem'\\''bin/remem' observe --host claude-code"
    );
}

#[test]
fn remove_remem_hooks_preserves_other_hooks() {
    let mut settings = json!({
        "hooks": {
            "SessionStart": [
                {"hooks": [{"command": "/tmp/remem context"}]},
                {"hooks": [{"command": "other-tool prepare"}]}
            ],
            "Stop": [
                {"hooks": [{"command": "remem summarize"}]}
            ]
        }
    });

    remove_remem_hooks(&mut settings, "/tmp/remem");

    assert_eq!(
        settings["hooks"]["SessionStart"]
            .as_array()
            .map(|arr| arr.len()),
        Some(1)
    );
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "other-tool prepare"
    );
    assert!(settings["hooks"].get("Stop").is_none());
}

#[test]
fn remove_remem_mcp_removes_named_and_command_matched_servers() {
    let mut settings = json!({
        "mcpServers": {
            "remem": {"command": "/tmp/remem", "args": ["mcp"]},
            "shadow": {"command": "/tmp/remem-alt", "args": []},
            "keep": {"command": "/usr/bin/other", "args": []}
        }
    });

    remove_remem_mcp(&mut settings, "/tmp/remem");

    assert!(settings["mcpServers"].get("remem").is_none());
    assert!(settings["mcpServers"].get("shadow").is_none());
    assert_eq!(settings["mcpServers"]["keep"]["command"], "/usr/bin/other");
}
