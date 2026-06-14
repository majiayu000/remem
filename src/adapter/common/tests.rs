use crate::db::test_support::ScopedTestDataDir;

use super::{
    event_summary, hook_payload_preview_redaction_input, parse_tool_hook, redact_and_truncate,
    redact_hook_payload_preview, redact_sensitive_text, redact_token, should_skip_bash_command,
    should_skip_tool, HOOK_PAYLOAD_PREVIEW_REDACTION_LOOKAHEAD_BYTES,
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
fn parses_reference_time_from_hook_timestamp() {
    let event = parse_tool_hook(
        r#"{
            "session_id":"s1",
            "cwd":"/tmp/remem",
            "timestamp":"2026-06-12T00:00:01.000Z",
            "tool_name":"Edit"
        }"#,
    )
    .expect("timestamped hook should parse");

    assert_eq!(event.reference_time_epoch, Some(1_781_222_401));
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
fn parse_error_log_redacts_payload_before_truncating() {
    let scoped = ScopedTestDataDir::new("adapter-parse-redact");
    unsafe {
        std::env::set_var("REMEM_STDERR_TO_LOG", "1");
    }

    let payload = format!(
        r#"{{"session_id":"s","tool_input":{{"command":"curl -H 'Authorization: Bearer ghp_1234567890abcdef'"}},"padding":"{}""#,
        "x".repeat(2_000)
    );
    let result = parse_tool_hook(&payload);
    assert!(result.is_none());

    let log = read_log_tail(&scoped);
    assert!(log.contains("[ERROR]"));
    assert!(
        log.contains("[REDACTED]"),
        "secret should be visibly redacted: {log}"
    );
    assert!(
        !log.contains("ghp_1234567890abcdef"),
        "raw token must not be logged: {log}"
    );

    unsafe {
        std::env::remove_var("REMEM_STDERR_TO_LOG");
    }
}

#[test]
fn hook_payload_preview_redacts_valid_sensitive_json_fields() {
    let payload = serde_json::json!({
        "session_id": "s",
        "tool_input": {
            "api_key": "sk-proj-12345678",
            "command": "curl -H 'Authorization: Bearer ghp_1234567890abcdef'"
        }
    })
    .to_string();

    let preview = redact_hook_payload_preview(&payload, 1_000);

    assert!(preview.contains("[REDACTED]"));
    assert!(!preview.contains("sk-proj-12345678"));
    assert!(!preview.contains("ghp_1234567890abcdef"));
}

#[test]
fn hook_payload_preview_redacts_malformed_inline_sensitive_assignments() {
    let payload = r#"{"session_id":"s","api_key":"short-secret","token=plain-short"#;

    let preview = redact_hook_payload_preview(payload, 1_000);

    assert!(preview.contains("[REDACTED]"));
    assert!(preview.contains(r#""session_id":"s""#));
    assert!(!preview.contains("short-secret"));
    assert!(!preview.contains("plain-short"));
}

#[test]
fn text_redaction_catches_command_key_assignments_with_short_values() {
    let redacted = redact_hook_payload_preview(
        "echo api_key=short-secret authorization=Bearer tiny-token",
        1_000,
    );

    assert!(redacted.contains("[REDACTED]"));
    assert!(!redacted.contains("short-secret"));
    assert!(!redacted.contains("tiny-token"));
}

#[test]
fn hook_payload_preview_bounds_redaction_work_for_large_malformed_payloads() {
    let payload = format!("api_key=short-secret {}", "x".repeat(20_000));
    let input = hook_payload_preview_redaction_input(&payload, 512);

    assert!(input.len() <= 512 + HOOK_PAYLOAD_PREVIEW_REDACTION_LOOKAHEAD_BYTES);
    assert!(input.contains("api_key=short-secret"));

    let redacted = redact_hook_payload_preview(&payload, 512);
    assert!(redacted.contains("[REDACTED]"));
    assert!(!redacted.contains("short-secret"));
}

#[test]
fn hook_payload_preview_redacts_escaped_quotes_in_malformed_sensitive_values() {
    let payload = r#"{"session_id":"s","password":"abc\"rest-of-secret","padding":"x"#;

    let redacted = redact_hook_payload_preview(payload, 1_000);

    assert!(redacted.contains("[REDACTED]"));
    assert!(!redacted.contains("abc"));
    assert!(!redacted.contains("rest-of-secret"));
}

#[test]
fn hook_payload_preview_redacts_multiline_malformed_sensitive_values() {
    let payload =
        "{\"session_id\":\"s\",\"password\":\"first-line\nsecond-line-secret\",\"safe\":\"visible\"";

    let redacted = redact_hook_payload_preview(payload, 1_000);

    assert!(redacted.contains("[REDACTED]"));
    assert!(!redacted.contains("first-line"));
    assert!(!redacted.contains("second-line-secret"));
    assert!(redacted.contains("visible"));
}

#[test]
fn hook_payload_preview_redacts_full_cookie_header_values() {
    let redacted = redact_hook_payload_preview(
        "curl -H 'Cookie: sid=abc; csrf=short' https://example.test",
        1_000,
    );

    assert!(redacted.contains("Cookie: [REDACTED]"));
    assert!(!redacted.contains("sid=abc"));
    assert!(!redacted.contains("csrf=short"));
    assert!(redacted.contains("https://example.test"));
}

#[test]
fn hook_payload_preview_redacts_basic_authorization_credentials() {
    let redacted = redact_hook_payload_preview(
        "curl -H 'Authorization: Basic dXNlcjpw' https://example.test",
        1_000,
    );

    assert!(redacted.contains("Authorization: [REDACTED]"));
    assert!(!redacted.contains("Basic dXNlcjpw"));
    assert!(!redacted.contains("dXNlcjpw"));
    assert!(redacted.contains("https://example.test"));
}

#[test]
fn hook_payload_preview_redacts_camel_case_secret_keys() {
    let payload = serde_json::json!({
        "accessToken": "short-secret",
        "clientSecret": "tiny-secret",
        "safe": "visible"
    })
    .to_string();

    let redacted = redact_hook_payload_preview(&payload, 1_000);

    assert!(redacted.contains("[REDACTED]"));
    assert!(redacted.contains("visible"));
    assert!(!redacted.contains("short-secret"));
    assert!(!redacted.contains("tiny-secret"));
}

#[test]
fn hook_payload_preview_redacts_sensitive_option_assignments() {
    let redacted = redact_hook_payload_preview(
        "curl --auth=short-secret --private-key=private.pem https://example.test",
        1_000,
    );

    assert!(redacted.contains("--auth=[REDACTED]"));
    assert!(
        redacted.contains("--private-key=[REDACTED]"),
        "unexpected redaction: {redacted}"
    );
    assert!(!redacted.contains("short-secret"));
    assert!(!redacted.contains("private.pem"));
}

#[test]
fn hook_payload_preview_redacts_sensitive_option_arguments() {
    let redacted = redact_hook_payload_preview(
        "curl -s -u alice:pw --oauth2-bearer tiny-token http://localhost",
        1_000,
    );

    assert!(redacted.contains("-u [REDACTED]"));
    assert!(redacted.contains("--oauth2-bearer [REDACTED]"));
    assert!(!redacted.contains("alice:pw"));
    assert!(!redacted.contains("tiny-token"));
    assert!(redacted.contains("http://localhost"));
}

#[test]
fn hook_payload_preview_redacts_url_userinfo_credentials() {
    let redacted =
        redact_hook_payload_preview("curl -s https://alice:pw@example.test/path?debug=1", 1_000);

    assert!(
        redacted.contains("https://[REDACTED]@example.test/path?debug=1"),
        "unexpected redaction: {redacted}"
    );
    assert!(!redacted.contains("alice:pw"));
}

#[test]
fn general_sensitive_text_redaction_does_not_use_hook_inline_heuristic() {
    let source = "let token = lexer.next_token();\nlet auth = AuthState::Anonymous;";

    assert_eq!(redact_sensitive_text(source), source);
}

#[test]
fn general_sensitive_text_redaction_does_not_treat_bare_words_as_options() {
    let source = "please pass the user value through without changing this phrase\nu value";

    assert_eq!(redact_sensitive_text(source), source);
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
fn redaction_keeps_benign_words_containing_secret_prefix_fragments() {
    let text = "flask-app disk-backed risk-aware task-sketch";

    assert_eq!(redact_and_truncate(text, 200), text);
    assert_eq!(redact_token("--name=flask-app"), "--name=flask-app");
    assert_eq!(redact_token("disk-backed-cache"), "disk-backed-cache");
}

#[test]
fn redaction_catches_key_prefixes_after_shell_and_json_punctuation() {
    assert_eq!(redact_token("sk-proj-12345678"), "[REDACTED]");
    assert_eq!(redact_token("--api-key=sk-proj-12345678"), "[REDACTED]");
    assert_eq!(
        redact_token(r#""token":"ghp_1234567890abcdef""#),
        "[REDACTED]"
    );
    assert_eq!(
        redact_token("token='github_pat_1234567890_abcdEFGH'"),
        "[REDACTED]"
    );
    assert_eq!(redact_token("github_pat_secret"), "[REDACTED]");
    assert_eq!(
        redact_token("Authorization=Bearer:xoxb-1234567890-abcdefghi"),
        "[REDACTED]"
    );
}
