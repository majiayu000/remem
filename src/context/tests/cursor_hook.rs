//! Cursor context stdout serialization tests (GH-823 B-003, B-004, B-005).
//!
//! These prove the serialization contract only. The Cursor 3.12.17
//! sessionStart injection capability itself stays disabled (PR #914 absent
//! marker), and GH-823 v1 has no postToolUse context command/renderer.

use super::super::host::HostKind;
use super::super::invocation::ContextInvocation;
use super::super::render::context_stdout_for_invocation;

fn cursor_invocation() -> ContextInvocation {
    ContextInvocation {
        cwd: "/tmp/remem-cursor".to_string(),
        project: "/tmp/remem-cursor".to_string(),
        session_id: Some("sess-cursor-1".to_string()),
        transcript_path: None,
        source: Some("sessionStart".to_string()),
        host: HostKind::Cursor,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: None,
    }
}

#[test]
fn cursor_session_start_emits_single_additional_context_object() {
    let stdout = context_stdout_for_invocation("remem context body", &cursor_invocation())
        .expect("serialization succeeds");

    let value: serde_json::Value =
        serde_json::from_str(stdout.trim_end()).expect("stdout is one JSON object");
    let object = value.as_object().expect("top-level object");
    assert_eq!(object.len(), 1, "only additional_context is emitted");
    assert_eq!(
        object.get("additional_context").and_then(|v| v.as_str()),
        Some("remem context body")
    );
    assert!(stdout.ends_with('\n'));
    assert!(
        !stdout.contains("hookSpecificOutput"),
        "cursor output must not use the Codex/Claude hook shape"
    );
}

#[test]
fn cursor_output_strips_ansi_sequences() {
    let stdout =
        context_stdout_for_invocation("\x1b[1;36mremem context\x1b[0m", &cursor_invocation())
            .expect("serialization succeeds");
    let value: serde_json::Value = serde_json::from_str(stdout.trim_end()).expect("json");
    assert_eq!(
        value["additional_context"].as_str(),
        Some("remem context"),
        "ANSI escapes must be stripped from the payload"
    );
}

#[test]
fn cursor_empty_body_emits_empty_stdout() {
    let stdout =
        context_stdout_for_invocation("", &cursor_invocation()).expect("empty body succeeds");
    assert!(stdout.is_empty(), "B-005: empty body means empty stdout");
}

#[test]
fn cursor_payload_contains_no_gh668_instruction_markers() {
    let body = "## Recent memory\n- decision: keep queue runner\n";
    let stdout =
        context_stdout_for_invocation(body, &cursor_invocation()).expect("serialization succeeds");
    // GH668 failure class: prompt-level control instructions asking the
    // assistant to re-render status lines or apply first-response workarounds.
    for marker in [
        "render exactly one status line",
        "status line",
        "first response",
        "first-response",
        "hidden directive",
        "you must",
        "You must",
    ] {
        assert!(
            !stdout.contains(marker),
            "cursor payload must not contain instruction marker '{marker}'"
        );
    }
}

#[test]
fn non_cursor_hosts_never_receive_cursor_shape() {
    for host in [HostKind::ClaudeCode, HostKind::Unknown] {
        let mut invocation = cursor_invocation();
        invocation.host = host;
        let stdout =
            context_stdout_for_invocation("body", &invocation).expect("serialization succeeds");
        assert_eq!(stdout, "body", "host {host:?} keeps plain stdout");
    }
}

#[test]
fn cursor_shape_requires_session_start_source() {
    let mut invocation = cursor_invocation();
    invocation.source = None;
    let stdout =
        context_stdout_for_invocation("body", &invocation).expect("serialization succeeds");
    assert_eq!(
        stdout, "body",
        "cursor JSON shape is gated on the strict sessionStart invocation"
    );
}
