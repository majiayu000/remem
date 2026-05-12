use crate::db::test_support::ScopedTestDataDir;

use super::{
    event_summary, parse_tool_hook, pending_tool_input, pending_tool_response,
    should_skip_bash_command, should_skip_tool,
};

fn read_log_tail(scoped: &ScopedTestDataDir) -> String {
    let log_path = scoped.path.join("remem.log");
    std::fs::read_to_string(&log_path).unwrap_or_default()
}

#[test]
fn malformed_json_returns_none_and_logs_error() {
    let scoped = ScopedTestDataDir::new("adapter-parse-malformed");
    // Route stderr through the log file so the test reads a single source.
    unsafe {
        std::env::set_var("REMEM_STDERR_TO_LOG", "1");
    }

    let result = parse_tool_hook("not-valid-json{");
    assert!(result.is_none(), "malformed JSON must not parse");

    let log = read_log_tail(&scoped);
    assert!(
        log.contains("[ERROR]") && log.contains("adapter") && log.contains("parse hook payload"),
        "expected error-level adapter parse failure log, got: {log}"
    );

    unsafe {
        std::env::remove_var("REMEM_STDERR_TO_LOG");
    }
}

#[test]
fn valid_json_with_missing_session_id_returns_none_silently() {
    let scoped = ScopedTestDataDir::new("adapter-parse-missing-session");
    unsafe {
        std::env::set_var("REMEM_STDERR_TO_LOG", "1");
    }

    // Valid JSON, but no session_id — this is the "different adapter shape"
    // path and must NOT log an error (only a true parse failure should).
    let result = parse_tool_hook(r#"{"tool_name":"Edit"}"#);
    assert!(result.is_none());

    let log = read_log_tail(&scoped);
    assert!(
        !log.contains("parse hook payload"),
        "missing session_id must not surface as a parse error, got: {log}"
    );

    unsafe {
        std::env::remove_var("REMEM_STDERR_TO_LOG");
    }
}

#[test]
fn truncates_long_payload_in_error_log() {
    let scoped = ScopedTestDataDir::new("adapter-parse-truncate");
    unsafe {
        std::env::set_var("REMEM_STDERR_TO_LOG", "1");
    }

    // Build invalid JSON that is much larger than the 512-char truncation window.
    let payload = format!("not-json-{}", "x".repeat(2_000));
    let result = parse_tool_hook(&payload);
    assert!(result.is_none());

    let log = read_log_tail(&scoped);
    assert!(log.contains("[ERROR]"));
    // The full 2000-char payload must not appear verbatim.
    assert!(
        !log.contains(&"x".repeat(1_000)),
        "raw payload should be truncated in the error log"
    );

    unsafe {
        std::env::remove_var("REMEM_STDERR_TO_LOG");
    }
}

#[test]
fn grep_and_glob_are_captured_as_search_events() {
    assert!(!should_skip_tool("Grep"));
    assert!(!should_skip_tool("Glob"));

    let grep_summary = event_summary(
        "Grep",
        &Some(serde_json::json!({
            "pattern": "PendingObservation",
            "path": "/Users/apple/Desktop/code/AI/tool/remem/src"
        })),
        &None,
    )
    .expect("Grep should classify");
    assert_eq!(grep_summary.event_type, "search");
    assert!(grep_summary.summary.contains("PendingObservation"));

    let glob_summary = event_summary(
        "Glob",
        &Some(serde_json::json!({"pattern": "src/**/*.rs"})),
        &None,
    )
    .expect("Glob should classify");
    assert_eq!(glob_summary.event_type, "search");
    assert!(glob_summary.summary.contains("src/**/*.rs"));
}

#[test]
fn targeted_bash_search_commands_are_captured() {
    assert!(!should_skip_bash_command(
        "rg -n \"event_summary\" src/adapter"
    ));
    assert!(!should_skip_bash_command(
        "rg -e \"event_summary\" src/adapter"
    ));
    assert!(!should_skip_bash_command(
        "grep -R \"enqueue_pending\" src/observe"
    ));
    assert!(!should_skip_bash_command(
        "grep -e \"enqueue_pending\" src/observe"
    ));
    assert!(!should_skip_bash_command("find src/adapter -name '*.rs'"));
    assert!(!should_skip_bash_command("git grep -n event_summary"));

    let summary = event_summary(
        "Bash",
        &Some(serde_json::json!({"command": "rg -n \"event_summary\" src/adapter"})),
        &Some(serde_json::json!({"exitCode": 0, "stdout": "src/adapter/common.rs:140:pub(crate) fn event_summary"})),
    )
    .expect("targeted search Bash should classify");
    assert_eq!(summary.event_type, "search");
    assert!(summary.summary.contains("Search `rg -n"));
}

#[test]
fn noisy_commands_still_skip() {
    assert!(should_skip_bash_command("git status --short"));
    assert!(should_skip_bash_command("ls -la"));
    assert!(should_skip_bash_command("cargo check"));
    assert!(should_skip_bash_command(
        "curl -s http://localhost:9800/tasks/1"
    ));
    assert!(should_skip_bash_command("ps aux | grep remem"));
    assert!(should_skip_bash_command("rg event_summary"));
    assert!(should_skip_bash_command("find . -name '*.rs'"));
}

#[test]
fn discovery_pending_payload_keeps_bounded_metadata_not_raw_output() {
    let input = Some(serde_json::json!({"command": "rg -n sk-secret src/adapter"}));
    let long_stdout = format!(
        "src/a.rs:1:{}\n{}",
        "A".repeat(64),
        "src/b.rs:2:".repeat(200)
    );
    let response = Some(serde_json::json!({
        "exitCode": 0,
        "stdout": long_stdout,
        "stderr": format!("warning token sk-{} {}", "x".repeat(80), "E".repeat(600))
    }));

    let tool_input = pending_tool_input("Bash", &input).expect("input payload");
    assert!(tool_input.contains("[REDACTED]"));
    assert!(!tool_input.contains("sk-secret"));

    let tool_response = pending_tool_response("Bash", &input, &response).expect("response payload");
    assert!(tool_response.contains("bounded_search_metadata"));
    assert!(tool_response.contains("stdout_bytes"));
    assert!(tool_response.contains("stderr_preview"));
    assert!(!tool_response.contains("src/a.rs:1"));
    assert!(!tool_response.contains(&"E".repeat(300)));
    assert!(!tool_response.contains("sk-"));
}

#[test]
fn grep_glob_pending_payload_uses_metadata_only_response() {
    let input = Some(serde_json::json!({"pattern": "session_id", "path": "src"}));
    let response = Some(serde_json::json!({
        "files": ["src/adapter/common.rs", "src/observe/hook.rs"],
        "output": "src/adapter/common.rs: session_id\nsrc/observe/hook.rs: session_id"
    }));

    let tool_input = pending_tool_input("Grep", &input).expect("grep input payload");
    assert!(tool_input.contains("session_id"));
    assert!(tool_input.contains("src"));

    let tool_response = pending_tool_response("Grep", &input, &response).expect("grep response");
    assert!(tool_response.contains("files_count"));
    assert!(tool_response.contains("output_bytes"));
    assert!(!tool_response.contains("src/adapter/common.rs: session_id"));
}
