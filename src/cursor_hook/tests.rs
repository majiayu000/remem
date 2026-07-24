//! Unit tests for the Cursor hook boundary (GH-823 SP823-T3/T4).

use serde_json::json;

use super::identity::normalize_workspace_root;
use super::input::{
    parse_observe_event, parse_session_start, read_bounded_hook_input, require_stop_event,
    CursorToolOutcome,
};
use super::{CURSOR_HOOK_STDIN_MAX_BYTES, CURSOR_TOOL_FIELD_MAX_BYTES};

/// Unique PII sentinel placed into every fixture; must never appear in
/// parsed events (beyond canonical fields) or in error messages (B-014).
pub(super) const EMAIL_SENTINEL: &str = "gh823.sentinel+cursor@example.invalid";

pub(super) fn session_start_fixture() -> serde_json::Value {
    json!({
        "conversation_id": "sess-cursor-1",
        "generation_id": "gen-1",
        "model": "auto",
        "model_id": "model-1",
        "model_params": [],
        "is_background_agent": false,
        "composer_mode": "agent",
        "session_id": "sess-cursor-1",
        "hook_event_name": "sessionStart",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": null
    })
}

pub(super) fn post_tool_use_fixture(tool_name: &str) -> serde_json::Value {
    json!({
        "conversation_id": "sess-cursor-1",
        "generation_id": "gen-1",
        "model": "auto",
        "tool_name": tool_name,
        "tool_input": {"file_path": "README.md"},
        "tool_output": "file contents",
        "duration": 12,
        "tool_use_id": "tu-1",
        "session_id": "sess-cursor-1",
        "hook_event_name": "postToolUse",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": "/tmp/transcript.jsonl"
    })
}

pub(super) fn post_tool_failure_fixture() -> serde_json::Value {
    json!({
        "conversation_id": "sess-cursor-1",
        "generation_id": "gen-1",
        "model": "auto",
        "tool_name": "Read",
        "tool_input": {"file_path": "missing.md"},
        "error_message": "file not found",
        "failure_type": "error",
        "duration": 8,
        "tool_use_id": "tu-fail-1",
        "is_interrupt": false,
        "session_id": "sess-cursor-1",
        "hook_event_name": "postToolUseFailure",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": "/tmp/transcript.jsonl"
    })
}

pub(super) fn stop_fixture(status: &str) -> serde_json::Value {
    json!({
        "conversation_id": "sess-cursor-1",
        "generation_id": "gen-1",
        "model": "auto",
        "model_id": "model-1",
        "model_params": [],
        "status": status,
        "loop_count": 0,
        "session_id": "sess-cursor-1",
        "hook_event_name": "stop",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": "/tmp/transcript.jsonl"
    })
}

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_string(value)
        .expect("fixture serializes")
        .into_bytes()
}

fn assert_no_sentinel(error: &anyhow::Error) {
    let rendered = format!("{error:#}");
    assert!(
        !rendered.contains(EMAIL_SENTINEL),
        "error message leaked the PII sentinel: {rendered}"
    );
    assert!(
        !rendered.contains("file contents"),
        "error message leaked payload content: {rendered}"
    );
}

// ---- bounded stdin reader (B-009) ----

#[test]
fn bounded_reader_accepts_exact_limit() {
    let data = vec![b' '; 64];
    let read = read_bounded_hook_input(&mut data.as_slice(), 64).expect("exact limit passes");
    assert_eq!(read.len(), 64);
}

#[test]
fn bounded_reader_rejects_one_byte_over_with_size_only_error() {
    let data = vec![b'q'; 65];
    let error = read_bounded_hook_input(&mut data.as_slice(), 64).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("limit=64"), "missing bound: {rendered}");
    assert!(
        rendered.contains("correlation_id="),
        "missing id: {rendered}"
    );
    assert!(!rendered.contains("qq"), "raw bytes leaked: {rendered}");
}

#[test]
fn bounded_reader_default_limit_is_frozen_value() {
    assert_eq!(CURSOR_HOOK_STDIN_MAX_BYTES, 1_048_576);
    let data = vec![b' '; CURSOR_HOOK_STDIN_MAX_BYTES + 1];
    let error =
        read_bounded_hook_input(&mut data.as_slice(), CURSOR_HOOK_STDIN_MAX_BYTES).unwrap_err();
    assert!(error.to_string().contains("limit=1048576"));
}

// ---- sessionStart parsing (B-002, B-003, B-013, B-014) ----

#[test]
fn session_start_parses_valid_pr914_shape() {
    let event = parse_session_start(&bytes(&session_start_fixture())).expect("valid fixture");
    assert_eq!(event.session_id, "sess-cursor-1");
    assert_eq!(event.workspace_root, "/tmp/remem-cursor");
    assert_eq!(event.transcript_path, None);
    let debug = format!("{event:?}");
    assert!(!debug.contains(EMAIL_SENTINEL), "sanitized event kept PII");
}

#[test]
fn session_start_accepts_string_transcript_path() {
    let mut fixture = session_start_fixture();
    fixture["transcript_path"] = json!("/tmp/t.jsonl");
    let event = parse_session_start(&bytes(&fixture)).expect("string path valid");
    assert_eq!(event.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
}

#[test]
fn session_start_rejects_identity_mismatch() {
    let mut fixture = session_start_fixture();
    fixture["conversation_id"] = json!("different-session");
    let error = parse_session_start(&bytes(&fixture)).unwrap_err();
    assert!(error.to_string().contains("identity equality"));
    assert_no_sentinel(&error);
}

#[test]
fn session_start_rejects_missing_blank_or_wrong_typed_session_id() {
    for value in [json!(null), json!(""), json!("   "), json!(42)] {
        let mut fixture = session_start_fixture();
        fixture["session_id"] = value.clone();
        fixture["conversation_id"] = value;
        let error = parse_session_start(&bytes(&fixture)).unwrap_err();
        assert_no_sentinel(&error);
    }
}

#[test]
fn session_start_rejects_every_invalid_workspace_root_shape() {
    let invalid_roots = [
        json!([]),
        json!([""]),
        json!(["   "]),
        json!(["", "/repo"]),
        json!(["/repo", ""]),
        json!(["/repo-a", "/repo-b"]),
        json!("not-an-array"),
        json!([42]),
    ];
    for roots in invalid_roots {
        let mut fixture = session_start_fixture();
        fixture["workspace_roots"] = roots.clone();
        let error = parse_session_start(&bytes(&fixture)).unwrap_err();
        assert_no_sentinel(&error);
        let rendered = error.to_string();
        assert!(
            rendered.contains("workspace_roots"),
            "unexpected error for {roots}: {rendered}"
        );
    }
}

#[test]
fn unverified_platform_root_shapes_fail_closed() {
    for root in [
        "/c:/repo",
        "C:\\repo",
        "\\\\server\\share",
        "relative/path",
        "//unc/share",
    ] {
        let error = normalize_workspace_root(root).unwrap_err();
        assert!(
            error.to_string().contains("unverified platform shape"),
            "root {root} should fail closed"
        );
        let mut fixture = session_start_fixture();
        fixture["workspace_roots"] = json!([root]);
        assert!(parse_session_start(&bytes(&fixture)).is_err());
    }
}

#[test]
fn session_start_rejects_event_command_mismatch() {
    for event_name in [
        "postToolUse",
        "stop",
        "beforeSubmitPrompt",
        "SessionStart",
        "unknown",
    ] {
        let mut fixture = session_start_fixture();
        fixture["hook_event_name"] = json!(event_name);
        let error = parse_session_start(&bytes(&fixture)).unwrap_err();
        assert_no_sentinel(&error);
    }
}

#[test]
fn malformed_payloads_fail_without_content_leak() {
    for payload in [
        &b"not-json"[..],
        &b"[1,2,3]"[..],
        &b"\"string\""[..],
        &[0xff, 0xfe, 0x00][..],
        &b""[..],
    ] {
        let error = parse_session_start(payload).unwrap_err();
        let rendered = error.to_string();
        assert!(!rendered.contains("not-json"), "leaked payload: {rendered}");
        assert!(rendered.contains("correlation_id="));
    }
}

// ---- observe parsing (B-007, B-015, B-016) ----

#[test]
fn observe_parses_observed_generic_success_tools() {
    for tool in ["Read", "Shell", "MCP:browser_tabs", "SomethingNew"] {
        let event = parse_observe_event(&bytes(&post_tool_use_fixture(tool)))
            .unwrap_or_else(|error| panic!("tool {tool} should parse: {error}"));
        assert_eq!(event.tool_name, tool, "verbatim tool name preserved");
        assert_eq!(event.tool_use_id, "tu-1");
        assert!(!event.outcome.is_failure());
        assert!(!format!("{event:?}").contains(EMAIL_SENTINEL));
    }
}

#[test]
fn observe_rejects_unobserved_success_variants_fail_closed() {
    for tool in ["Task", "Write", "Edit", "Delete"] {
        let error = parse_observe_event(&bytes(&post_tool_use_fixture(tool))).unwrap_err();
        assert!(
            error.to_string().contains("fail-closed"),
            "tool {tool} must stay fail-closed"
        );
    }
}

#[test]
fn observe_requires_non_empty_string_tool_use_id() {
    for value in [json!(null), json!(""), json!("  "), json!(7), json!(["tu"])] {
        let mut fixture = post_tool_use_fixture("Read");
        fixture["tool_use_id"] = value;
        let error = parse_observe_event(&bytes(&fixture)).unwrap_err();
        assert!(error.to_string().contains("tool_use_id"));
    }
    let mut fixture = post_tool_use_fixture("Read");
    fixture
        .as_object_mut()
        .expect("object")
        .remove("tool_use_id");
    assert!(parse_observe_event(&bytes(&fixture)).is_err());
}

#[test]
fn observe_requires_object_input_and_string_output() {
    let mut fixture = post_tool_use_fixture("Read");
    fixture["tool_input"] = json!("stringified");
    assert!(parse_observe_event(&bytes(&fixture)).is_err());

    let mut fixture = post_tool_use_fixture("Read");
    fixture["tool_output"] = json!({"content": "obj"});
    assert!(parse_observe_event(&bytes(&fixture)).is_err());

    let mut fixture = post_tool_use_fixture("Read");
    fixture
        .as_object_mut()
        .expect("object")
        .remove("tool_output");
    assert!(parse_observe_event(&bytes(&fixture)).is_err());
}

#[test]
fn observe_requires_required_conversation_id() {
    let mut fixture = post_tool_use_fixture("Read");
    fixture
        .as_object_mut()
        .expect("object")
        .remove("conversation_id");
    let error = parse_observe_event(&bytes(&fixture)).unwrap_err();
    assert!(error.to_string().contains("conversation_id"));
}

#[test]
fn observe_accepts_distinct_subagent_identity_without_parent_coercion() {
    let mut fixture = post_tool_use_fixture("Read");
    fixture["session_id"] = json!("subagent-7");
    fixture["conversation_id"] = json!("subagent-7");
    fixture["transcript_path"] = json!(null);
    let event = parse_observe_event(&bytes(&fixture)).expect("child identity valid");
    assert_eq!(event.session_id, "subagent-7");
    assert_eq!(event.transcript_path, None, "null child path stays null");
}

#[test]
fn observe_tool_output_boundary_exact_and_one_byte_over() {
    let mut fixture = post_tool_use_fixture("Read");
    fixture["tool_output"] = json!("y".repeat(CURSOR_TOOL_FIELD_MAX_BYTES));
    parse_observe_event(&bytes(&fixture)).expect("exact-limit output accepted");

    let mut fixture = post_tool_use_fixture("Read");
    fixture["tool_output"] = json!("y".repeat(CURSOR_TOOL_FIELD_MAX_BYTES + 1));
    let error = parse_observe_event(&bytes(&fixture)).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("tool_output"));
    assert!(rendered.contains(&format!("limit={CURSOR_TOOL_FIELD_MAX_BYTES}")));
    assert!(!rendered.contains("yyy"), "payload bytes leaked");
}

#[test]
fn observe_canonical_tool_input_boundary() {
    // Canonical serialization is {"pad":"<filler>"}: 10 bytes of framing.
    let framing = r#"{"pad":""}"#.len();
    let mut fixture = post_tool_use_fixture("Read");
    fixture["tool_input"] = json!({"pad": "z".repeat(CURSOR_TOOL_FIELD_MAX_BYTES - framing)});
    parse_observe_event(&bytes(&fixture)).expect("exact-limit canonical input accepted");

    let mut fixture = post_tool_use_fixture("Read");
    fixture["tool_input"] = json!({"pad": "z".repeat(CURSOR_TOOL_FIELD_MAX_BYTES - framing + 1)});
    let error = parse_observe_event(&bytes(&fixture)).unwrap_err();
    assert!(error.to_string().contains("tool_input"));
}

#[test]
fn observe_failure_accepts_only_observed_read_shape() {
    let event = parse_observe_event(&bytes(&post_tool_failure_fixture())).expect("failed Read");
    assert_eq!(event.tool_use_id, "tu-fail-1");
    match &event.outcome {
        CursorToolOutcome::Failure {
            error_message,
            failure_type,
            is_interrupt,
            ..
        } => {
            assert_eq!(error_message, "file not found");
            assert_eq!(failure_type, "error");
            assert!(!is_interrupt);
        }
        CursorToolOutcome::Success { .. } => panic!("failure fixture parsed as success"),
    }

    for tool in ["Shell", "Task", "Write", "MCP:browser_tabs", "SomethingNew"] {
        let mut fixture = post_tool_failure_fixture();
        fixture["tool_name"] = json!(tool);
        let error = parse_observe_event(&bytes(&fixture)).unwrap_err();
        assert!(
            error.to_string().contains("fail-closed"),
            "failure for {tool} must stay fail-closed"
        );
    }
}

#[test]
fn observe_failure_requires_exact_failure_fields() {
    let mut fixture = post_tool_failure_fixture();
    fixture["failure_type"] = json!("timeout");
    assert!(parse_observe_event(&bytes(&fixture)).is_err());

    let mut fixture = post_tool_failure_fixture();
    fixture
        .as_object_mut()
        .expect("object")
        .remove("is_interrupt");
    assert!(parse_observe_event(&bytes(&fixture)).is_err());

    let mut fixture = post_tool_failure_fixture();
    fixture["duration"] = json!("8");
    assert!(parse_observe_event(&bytes(&fixture)).is_err());
}

#[test]
fn observe_rejects_mcp_specific_events_under_generic_ownership() {
    for event_name in [
        "beforeMCPExecution",
        "afterMCPExecution",
        "preToolUse",
        "subagentStart",
    ] {
        let mut fixture = post_tool_use_fixture("MCP:browser_tabs");
        fixture["hook_event_name"] = json!(event_name);
        let error = parse_observe_event(&bytes(&fixture)).unwrap_err();
        assert!(
            error.to_string().contains(event_name),
            "event {event_name} must be rejected by name"
        );
    }
}

// ---- stop gate (B-008 pre-T5 slice) ----

#[test]
fn stop_event_name_is_validated_before_fail_closed_gate() {
    require_stop_event(&bytes(&stop_fixture("completed"))).expect("stop recognized");
    require_stop_event(&bytes(&stop_fixture("aborted"))).expect("stop recognized");
    for event_name in ["postToolUse", "sessionStart", "Stop"] {
        let mut fixture = stop_fixture("completed");
        fixture["hook_event_name"] = json!(event_name);
        assert!(require_stop_event(&bytes(&fixture)).is_err());
    }
}
