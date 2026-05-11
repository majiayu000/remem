use crate::db::test_support::ScopedTestDataDir;

use super::parse_tool_hook;

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
