//! Subprocess-level Cursor hook protocol tests (GH-823 B-001, B-006, B-009).
//!
//! Each test runs the real `remem` binary with an isolated `REMEM_DATA_DIR`
//! and proves exit status, stdout, and zero-side-effect behavior at the
//! process boundary — including the exact 1,048,576-byte stdin limit and the
//! one-byte-over rejection for every Cursor entrypoint.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EMAIL_SENTINEL: &str = "gh823.sentinel+cursor@example.invalid";
const STDIN_LIMIT: usize = 1_048_576;

struct HookRun {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn temp_data_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "remem-cursor-hooks-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp data dir");
    dir
}

fn run_hook(data_dir: &Path, args: &[&str], stdin_bytes: &[u8]) -> HookRun {
    let mut child = Command::new(env!("CARGO_BIN_EXE_remem"))
        .args(args)
        .env("REMEM_DATA_DIR", data_dir)
        .env("REMEM_ALLOW_PLAINTEXT_DB", "1")
        .env_remove("REMEM_DISABLE_HOOKS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn remem binary");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(stdin_bytes)
        .expect("write hook stdin");
    let output = child.wait_with_output().expect("collect output");
    HookRun {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn session_start_payload() -> String {
    serde_json::json!({
        "conversation_id": "sess-sub-1",
        "generation_id": "gen-1",
        "model": "auto",
        "session_id": "sess-sub-1",
        "hook_event_name": "sessionStart",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor-sub"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": null
    })
    .to_string()
}

fn post_tool_use_payload() -> String {
    serde_json::json!({
        "conversation_id": "sess-sub-1",
        "generation_id": "gen-1",
        "model": "auto",
        "tool_name": "Read",
        "tool_input": {"file_path": "README.md"},
        "tool_output": "file contents",
        "duration": 12,
        "tool_use_id": "tu-sub-1",
        "session_id": "sess-sub-1",
        "hook_event_name": "postToolUse",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor-sub"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": "/tmp/transcript.jsonl"
    })
    .to_string()
}

fn stop_payload() -> String {
    serde_json::json!({
        "conversation_id": "sess-sub-1",
        "generation_id": "gen-1",
        "status": "completed",
        "loop_count": 0,
        "session_id": "sess-sub-1",
        "hook_event_name": "stop",
        "cursor_version": "3.12.17",
        "workspace_roots": ["/tmp/remem-cursor-sub"],
        "user_email": EMAIL_SENTINEL,
        "transcript_path": "/tmp/transcript.jsonl"
    })
    .to_string()
}

static MIGRATE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Migrates a fresh remem database at `dir` the way an installed host would
/// have, by briefly pointing this process's `REMEM_DATA_DIR` at it. The env
/// mutation is serialized and restored; all hook behavior under test still
/// runs in subprocesses with explicit env.
fn migrate_db_at(dir: &Path) {
    let _guard = MIGRATE_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let previous_dir = std::env::var_os("REMEM_DATA_DIR");
    let previous_plaintext = std::env::var_os("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::set_var("REMEM_DATA_DIR", dir);
    std::env::set_var("REMEM_ALLOW_PLAINTEXT_DB", "1");
    let result = remem::db::open_db();
    match previous_dir {
        Some(value) => std::env::set_var("REMEM_DATA_DIR", value),
        None => std::env::remove_var("REMEM_DATA_DIR"),
    }
    match previous_plaintext {
        Some(value) => std::env::set_var("REMEM_ALLOW_PLAINTEXT_DB", value),
        None => std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB"),
    }
    drop(result.expect("migrate test database"));
}

/// Pads a JSON payload with trailing spaces (valid JSON whitespace) to an
/// exact byte length so exact-limit inputs still reach normal validation.
fn pad_to_exact(payload: &str, len: usize) -> Vec<u8> {
    assert!(payload.len() <= len, "payload longer than target");
    let mut bytes = payload.as_bytes().to_vec();
    bytes.resize(len, b' ');
    bytes
}

/// Asserts zero persistence side effects. Error-level diagnostic logs are
/// expected on fail-closed paths (B-009) and are ignored; any database,
/// spill, or queue file is a violation.
fn assert_dir_has_no_files(dir: &Path) {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .expect("read data dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with("remem.log"))
        .collect();
    assert!(
        entries.is_empty(),
        "expected zero persistence side effects, found files: {entries:?}"
    );
}

// ---- B-009: whole-stdin limit per entrypoint ----

#[test]
fn context_one_byte_over_stdin_fails_closed_with_zero_side_effects() {
    let dir = temp_data_dir("context-over");
    let run = run_hook(
        &dir,
        &["context", "--host", "cursor"],
        &vec![b' '; STDIN_LIMIT + 1],
    );
    assert!(!run.status.success(), "one byte over must exit non-zero");
    assert!(
        run.stdout.is_empty(),
        "stdout must stay empty: {}",
        run.stdout
    );
    assert!(
        run.stderr.contains("limit=1048576"),
        "size-only error expected: {}",
        run.stderr
    );
    assert!(!run.stderr.contains(EMAIL_SENTINEL));
    assert_dir_has_no_files(&dir);
}

#[test]
fn observe_one_byte_over_stdin_fails_closed_with_zero_side_effects() {
    let dir = temp_data_dir("observe-over");
    let payload = pad_to_exact(&post_tool_use_payload(), STDIN_LIMIT + 1);
    let run = run_hook(&dir, &["observe", "--host", "cursor"], &payload);
    assert!(!run.status.success());
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("limit=1048576"), "{}", run.stderr);
    assert_dir_has_no_files(&dir);
}

#[test]
fn summarize_one_byte_over_stdin_fails_closed_with_zero_side_effects() {
    let dir = temp_data_dir("summarize-over");
    let payload = pad_to_exact(&stop_payload(), STDIN_LIMIT + 1);
    let run = run_hook(&dir, &["summarize", "--host", "cursor"], &payload);
    assert!(!run.status.success());
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("limit=1048576"), "{}", run.stderr);
    assert_dir_has_no_files(&dir);
}

#[test]
fn observe_exact_limit_stdin_reaches_normal_validation_and_captures() {
    let dir = temp_data_dir("observe-exact");
    migrate_db_at(&dir);

    let payload = pad_to_exact(&post_tool_use_payload(), STDIN_LIMIT);
    let run = run_hook(&dir, &["observe", "--host", "cursor"], &payload);
    assert!(
        run.status.success(),
        "exact-limit observe failed: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "observe emits no stdout");

    let conn = rusqlite::Connection::open(dir.join("remem.db")).expect("open db");
    let (host, event_type): (String, String) = conn
        .query_row(
            "SELECT h.name, ce.event_type FROM captured_events ce
             JOIN hosts h ON h.id = ce.host_id
             WHERE ce.event_id = 'cursor-tool:tu-sub-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("captured row exists");
    assert_eq!(host, "cursor", "B-011 canonical host value");
    assert_eq!(event_type, "tool_result");
    let leaked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM captured_events WHERE content_text LIKE ?1",
            [format!("%{EMAIL_SENTINEL}%")],
            |row| row.get(0),
        )
        .expect("sentinel scan");
    assert_eq!(leaked, 0, "user_email sentinel must not reach the database");
}

// ---- B-001: closed-set host validation at every hook command ----

#[test]
fn hook_commands_reject_aliases_unknown_and_arbitrary_hosts() {
    let dir = temp_data_dir("alias-reject");
    for command in ["context", "observe", "summarize", "session-init"] {
        for host in ["claude", "codex", "unknown", "cursor-ide", "Cursor", ""] {
            let run = run_hook(&dir, &[command, "--host", host], b"{}");
            assert!(
                !run.status.success(),
                "{command} --host '{host}' must fail the closed-set parser"
            );
            assert!(
                run.stdout.is_empty(),
                "{command} --host '{host}' wrote stdout"
            );
            assert!(
                run.stderr.contains("claude-code, codex-cli, cursor"),
                "{command} --host '{host}' should enumerate the closed set: {}",
                run.stderr
            );
        }
    }
    assert_dir_has_no_files(&dir);
}

// ---- B-006: session-init is explicitly unsupported on cursor ----

#[test]
fn session_init_cursor_rejected_before_stdin_and_side_effects() {
    let dir = temp_data_dir("session-init-cursor");
    // A huge stdin proves rejection happens without consuming the payload
    // into prompt capture: the process must still exit quickly and cleanly.
    let run = run_hook(
        &dir,
        &["session-init", "--host", "cursor"],
        session_start_payload().as_bytes(),
    );
    assert!(
        !run.status.success(),
        "session-init cursor must exit non-zero"
    );
    assert!(run.stdout.is_empty(), "no context stdout may be produced");
    assert!(
        run.stderr.contains("not supported on --host cursor"),
        "explicit unsupported error expected: {}",
        run.stderr
    );
    assert_dir_has_no_files(&dir);
}

// ---- SP823-T5 + GH-825: cursor summarize Stop wiring ----

#[test]
fn summarize_cursor_unapproved_status_fails_closed_with_zero_side_effects() {
    let dir = temp_data_dir("summarize-cursor-status");
    let mut payload: serde_json::Value =
        serde_json::from_str(&stop_payload()).expect("stop payload parses");
    payload["status"] = serde_json::json!("error");
    let run = run_hook(
        &dir,
        &["summarize", "--host", "cursor"],
        payload.to_string().as_bytes(),
    );
    assert!(
        !run.status.success(),
        "unobserved status must fail closed: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("approved set"), "{}", run.stderr);
    assert!(!run.stderr.contains(EMAIL_SENTINEL));
    assert_dir_has_no_files(&dir);
}

#[test]
fn summarize_cursor_valid_stop_records_durable_marked_capture() {
    let dir = temp_data_dir("summarize-cursor-full");
    migrate_db_at(&dir);
    let transcript = dir.join("cursor-transcript.jsonl");
    std::fs::write(
        &transcript,
        concat!(
            "{\"role\":\"user\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ask\"}]}}\n",
            "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"answer\"}]}}\n",
            "{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
        ),
    )
    .expect("write transcript fixture");
    let mut payload: serde_json::Value =
        serde_json::from_str(&stop_payload()).expect("stop payload parses");
    payload["transcript_path"] = serde_json::json!(transcript.to_string_lossy());

    let run = run_hook(
        &dir,
        &["summarize", "--host", "cursor"],
        payload.to_string().as_bytes(),
    );
    assert!(run.status.success(), "valid stop failed: {}", run.stderr);
    assert!(run.stdout.is_empty(), "summarize emits no stdout");

    let conn = rusqlite::Connection::open(dir.join("remem.db")).expect("open db");
    let (host, content): (String, String) = conn
        .query_row(
            "SELECT h.name, ce.content_text FROM captured_events ce
             JOIN hosts h ON h.id = ce.host_id
             WHERE ce.event_type = 'session_stop'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("durable session_stop row exists");
    assert_eq!(host, "cursor");
    let stored: serde_json::Value = serde_json::from_str(&content).expect("stored payload JSON");
    assert!(
        stored.get("transcript_path").is_none(),
        "cursor Stop payload must never persist a transcript path"
    );
    let capture = stored["cursor_capture"].as_object().expect("marker");
    assert_eq!(capture["fidelity"], "full");
    assert_eq!(capture["stop_key"], "sess-sub-1:gen-1:0");
    assert_eq!(
        capture["snapshot"]["messages"]
            .as_array()
            .expect("ir")
            .len(),
        2
    );
    let leaked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM captured_events WHERE content_text LIKE ?1",
            [format!("%{EMAIL_SENTINEL}%")],
            |row| row.get(0),
        )
        .expect("sentinel scan");
    assert_eq!(leaked, 0, "user_email sentinel must not reach the database");
}

#[test]
fn summarize_cursor_missing_transcript_degrades_without_losing_the_stop() {
    let dir = temp_data_dir("summarize-cursor-degraded");
    migrate_db_at(&dir);
    let mut payload: serde_json::Value =
        serde_json::from_str(&stop_payload()).expect("stop payload parses");
    payload["transcript_path"] =
        serde_json::json!(dir.join("missing-transcript.jsonl").to_string_lossy());

    let run = run_hook(
        &dir,
        &["summarize", "--host", "cursor"],
        payload.to_string().as_bytes(),
    );
    assert!(run.status.success(), "degraded stop failed: {}", run.stderr);

    let conn = rusqlite::Connection::open(dir.join("remem.db")).expect("open db");
    let content: String = conn
        .query_row(
            "SELECT content_text FROM captured_events WHERE event_type = 'session_stop'",
            [],
            |row| row.get(0),
        )
        .expect("durable session_stop row exists");
    let stored: serde_json::Value = serde_json::from_str(&content).expect("stored payload JSON");
    assert_eq!(stored["cursor_capture"]["fidelity"], "degraded");
    assert_eq!(stored["cursor_capture"]["reason_code"], "read_failed");
    let drop_reason: String = conn
        .query_row(
            "SELECT reason FROM capture_drop_events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("degradation diagnostic exists");
    assert_eq!(drop_reason, "cursor_transcript_read_failed");
    let raw_path_leak: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM capture_drop_events WHERE detail LIKE '%missing-transcript%'",
            [],
            |row| row.get(0),
        )
        .expect("locator scan");
    assert_eq!(raw_path_leak, 0, "drop detail must not echo the raw path");
}

#[test]
fn summarize_cursor_rejects_event_command_mismatch() {
    let dir = temp_data_dir("summarize-mismatch");
    let run = run_hook(
        &dir,
        &["summarize", "--host", "cursor"],
        post_tool_use_payload().as_bytes(),
    );
    assert!(!run.status.success());
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("mismatch"), "{}", run.stderr);
    assert_dir_has_no_files(&dir);
}

// ---- B-002/B-003: context strict parsing at the process boundary ----

#[test]
fn context_cursor_emits_additional_context_json_or_nothing() {
    let dir = temp_data_dir("context-valid");
    migrate_db_at(&dir);

    let run = run_hook(
        &dir,
        &["context", "--host", "cursor"],
        session_start_payload().as_bytes(),
    );
    assert!(
        run.status.success(),
        "valid sessionStart failed: {}",
        run.stderr
    );
    if !run.stdout.is_empty() {
        let value: serde_json::Value =
            serde_json::from_str(run.stdout.trim_end()).expect("stdout must be one JSON object");
        let object = value.as_object().expect("object");
        assert_eq!(object.len(), 1);
        assert!(object.contains_key("additional_context"));
        assert!(!run.stdout.contains(EMAIL_SENTINEL));
        assert!(!run.stdout.contains("hookSpecificOutput"));
    }
}

#[test]
fn context_cursor_rejects_malformed_and_mismatched_payloads() {
    let dir = temp_data_dir("context-invalid");
    let identity_mismatch = {
        let mut value: serde_json::Value =
            serde_json::from_str(&session_start_payload()).expect("fixture json");
        value["conversation_id"] = serde_json::Value::String("other-session".to_string());
        value.to_string()
    };
    for payload in [
        "not-json".to_string(),
        post_tool_use_payload(),
        stop_payload(),
        identity_mismatch,
    ] {
        let run = run_hook(&dir, &["context", "--host", "cursor"], payload.as_bytes());
        assert!(!run.status.success(), "payload must fail closed: {payload}");
        assert!(run.stdout.is_empty(), "no stdout on failure");
        assert!(!run.stderr.contains(EMAIL_SENTINEL));
    }
    assert_dir_has_no_files(&dir);
}
