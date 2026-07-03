use super::super::host::HostKind;
use super::super::invocation::ContextInvocation;
use super::super::render::{context_stdout_for_invocation, empty_context_output};
use super::super::types::ContextRequest;

#[test]
fn empty_context_uses_ansi_when_color_enabled() {
    let output = empty_context_output(&ContextRequest {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: None,
        hook_source: Some("compact".to_string()),
        current_branch: Some("main".to_string()),
        host: HostKind::CodexCli,
        use_colors: true,
    });

    assert!(output.starts_with("\x1b[1;36mremem context\x1b[0m"));
    assert!(output.contains("\x1b[1;36mremem context\x1b[0m"));
    assert!(output.contains("├─ \x1b[1mproject\x1b[0m: /tmp/remem"));
}

#[test]
fn codex_colored_header_aligns_rows_under_hook_context_value() {
    let output = empty_context_output(&ContextRequest {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: None,
        hook_source: None,
        current_branch: Some("main".to_string()),
        host: HostKind::CodexCli,
        use_colors: true,
    });
    let plain = super::super::style::strip_ansi(&output);
    let mut lines = plain.lines();

    assert_eq!(lines.next(), Some("remem context"));
    let project_line = lines.next().unwrap_or_default();
    assert!(project_line.ends_with("├─ project: /tmp/remem"));
    let row_indent = project_line.chars().take_while(|ch| *ch == ' ').count();
    assert_eq!(row_indent, "hook context: ".chars().count());
}

#[test]
fn codex_session_start_hook_stdout_uses_structured_additional_context() {
    let invocation = ContextInvocation {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: Some("sess-hook-json".to_string()),
        transcript_path: Some("/tmp/remem/session.jsonl".to_string()),
        source: Some("startup".to_string()),
        host: HostKind::CodexCli,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: None,
    };
    let context = "remem context\nUse `search`/`get_observations` for details.\n";

    let stdout = context_stdout_for_invocation(context, &invocation).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert_eq!(
        parsed["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert_eq!(parsed["hookSpecificOutput"]["additionalContext"], context);
    assert!(!stdout.contains("first assistant response"));
    assert!(!stdout.contains("Remem context:"));
}

#[test]
fn codex_session_start_hook_stdout_strips_ansi_before_model_context() {
    let invocation = ContextInvocation {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: Some("sess-hook-color".to_string()),
        transcript_path: None,
        source: Some("compact".to_string()),
        host: HostKind::CodexCli,
        use_colors: true,
        debug: false,
        force: false,
        gate_mode: None,
    };
    let context = "\x1b[1;36mremem context\x1b[0m\nbody\n";

    let stdout = context_stdout_for_invocation(context, &invocation).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert_eq!(
        parsed["hookSpecificOutput"]["additionalContext"],
        "remem context\nbody\n"
    );
}

#[test]
fn codex_direct_context_stdout_stays_plain_text() {
    let invocation = ContextInvocation {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: None,
        transcript_path: None,
        source: None,
        host: HostKind::CodexCli,
        use_colors: false,
        debug: false,
        force: true,
        gate_mode: Some("off".to_string()),
    };

    let stdout = context_stdout_for_invocation("remem context\n", &invocation).unwrap();

    assert_eq!(stdout, "remem context\n");
}

#[test]
fn codex_suppressed_context_stdout_stays_silent() {
    let invocation = ContextInvocation {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: Some("sess-suppressed".to_string()),
        transcript_path: None,
        source: Some("compact".to_string()),
        host: HostKind::CodexCli,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: None,
    };

    let stdout = context_stdout_for_invocation("", &invocation).unwrap();

    assert_eq!(stdout, "");
}
