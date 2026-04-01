use crate::adapter::{ParsedHookEvent, ToolAdapter};

use super::{should_skip_bash_command, ClaudeCodeAdapter};

#[test]
fn skip_read_only_search_commands() {
    assert!(should_skip_bash_command("grep -rn \"foo\" src/"));
    assert!(should_skip_bash_command("rg -n foo src"));
    assert!(should_skip_bash_command("find src -name '*.ts'"));
    assert!(should_skip_bash_command("git grep -n startIngestionJob"));
}

#[test]
fn skip_read_only_polling_commands() {
    assert!(should_skip_bash_command(
        "curl -s http://localhost:9800/tasks/1"
    ));
    assert!(should_skip_bash_command(
        "sleep 60 && curl -s http://localhost:9800/tasks/1"
    ));
}

#[test]
fn keep_mutating_commands() {
    assert!(!should_skip_bash_command("git add src/observe.rs"));
    assert!(!should_skip_bash_command(
        "git commit -m \"feat: tune filter\""
    ));
    assert!(!should_skip_bash_command("git push origin main"));
    assert!(!should_skip_bash_command(
        "curl -X POST http://localhost:9800/tasks"
    ));
}

#[test]
fn classify_edit_event() {
    let adapter = ClaudeCodeAdapter;
    let event = ParsedHookEvent {
        session_id: "s1".into(),
        cwd: Some("/tmp".into()),
        project: "test".into(),
        tool_name: "Edit".into(),
        tool_input: Some(serde_json::json!({"file_path": "/Users/x/src/main.rs"})),
        tool_response: None,
    };
    let summary = adapter.classify_event(&event).unwrap();
    assert_eq!(summary.event_type, "file_edit");
    assert!(summary.summary.contains("main.rs"));
}

#[test]
fn should_skip_metadata_tools() {
    let adapter = ClaudeCodeAdapter;
    let event = ParsedHookEvent {
        session_id: "s1".into(),
        cwd: None,
        project: "test".into(),
        tool_name: "TodoWrite".into(),
        tool_input: None,
        tool_response: None,
    };
    assert!(adapter.should_skip(&event));
}
