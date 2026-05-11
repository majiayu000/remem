use crate::adapter::{ParsedHookEvent, ToolAdapter};

use super::CodexAdapter;

#[test]
fn parses_codex_tool_hook_shape() {
    let adapter = CodexAdapter;
    let json = r#"{
        "session_id": "s1",
        "cwd": "/tmp/remem",
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "python test.py"},
        "tool_result": {"exitCode": 0}
    }"#;

    let event = adapter.parse_hook(json).expect("codex hook should parse");

    assert_eq!(event.session_id, "s1");
    assert_eq!(event.cwd.as_deref(), Some("/tmp/remem"));
    assert_eq!(event.tool_name, "Bash");
    assert_eq!(event.tool_response.unwrap()["exitCode"], 0);
}

#[test]
fn skips_read_only_bash_with_shared_policy() {
    let adapter = CodexAdapter;
    assert!(adapter.should_skip_bash("git status --short"));
    assert!(!adapter.should_skip_bash("git push origin main"));
}

#[test]
fn classifies_bash_event() {
    let adapter = CodexAdapter;
    let event = ParsedHookEvent {
        session_id: "s1".into(),
        cwd: Some("/tmp".into()),
        project: "test".into(),
        tool_name: "Bash".into(),
        tool_input: Some(serde_json::json!({"command": "python test.py"})),
        tool_response: Some(serde_json::json!({"exitCode": 1, "stderr": "failed"})),
    };

    let summary = adapter.classify_event(&event).unwrap();

    assert_eq!(summary.event_type, "bash");
    assert_eq!(summary.exit_code, Some(1));
    assert_eq!(summary.detail.as_deref(), Some("failed"));
}
