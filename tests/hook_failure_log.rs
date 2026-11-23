//! Subprocess-level proof that a failed hook command explains itself in
//! `remem.log`.
//!
//! Hosts such as the DeepSeek Harness adapter discard hook stderr and send
//! the user to the remem log, so the terminal error of `session-init`,
//! `observe` and `summarize` must be written there, not only to stderr.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct HookRun {
    status: std::process::ExitStatus,
    stderr: String,
}

fn temp_data_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "remem-hook-failure-log-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp data dir");
    dir
}

/// Runs a hook against a data directory that was never initialized: no key
/// file, no database, and no plaintext override.
fn run_uninitialized_hook(binary: &str, data_dir: &Path, command: &str, payload: &str) -> HookRun {
    let mut child = Command::new(binary)
        .args([command, "--host", "deepseek-harness"])
        .env("REMEM_DATA_DIR", data_dir)
        .env("REMEM_CONFIG", data_dir.join("config.toml"))
        .env("HOME", data_dir)
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove("REMEM_DISABLE_HOOKS")
        .env_remove("REMEM_STDERR_TO_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn remem binary");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(payload.as_bytes())
        .expect("write hook stdin");
    let output = child.wait_with_output().expect("collect output");
    HookRun {
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn dsh_payloads(cwd: &Path) -> Vec<(&'static str, String)> {
    let base = serde_json::json!({
        "host": "deepseek-harness",
        "session_id": "hook-failure-log",
        "cwd": cwd,
        "reference_time_epoch": 1_763_300_000,
    });
    let with = |extra: serde_json::Value| {
        let mut payload = base.clone();
        payload
            .as_object_mut()
            .expect("base payload is an object")
            .extend(extra.as_object().expect("extra is an object").clone());
        payload.to_string()
    };
    vec![
        (
            "session-init",
            with(serde_json::json!({
                "hook_event_name": "UserPromptSubmit",
                "turn_id": "dsh:1",
                "prompt": "Remember: we use pnpm.",
            })),
        ),
        (
            "observe",
            with(serde_json::json!({
                "tool_name": "assistant/message",
                "tool_input": { "dsh_event_type": "assistant/message", "arguments": {} },
                "tool_response": { "text": "Noted." },
            })),
        ),
        (
            "summarize",
            with(serde_json::json!({
                "last_assistant_message": "Noted.",
                "reason": { "kind": "completed" },
            })),
        ),
    ]
}

#[test]
fn failed_hook_commands_record_their_cause_in_the_remem_log() {
    for (label, binary) in [
        ("full", env!("CARGO_BIN_EXE_remem")),
        ("slim", env!("CARGO_BIN_EXE_remem-hook")),
    ] {
        for (command, payload) in dsh_payloads(&std::env::temp_dir()) {
            let dir = temp_data_dir(&format!("{label}-{command}"));
            let run = run_uninitialized_hook(binary, &dir, command, &payload);
            assert!(
                !run.status.success(),
                "{label} {command} must fail without an initialized store"
            );
            assert!(
                run.stderr.contains("SQLCipher key"),
                "{label} {command} stderr should name the missing key: {}",
                run.stderr
            );
            assert!(
                !run.stderr.contains("hook command failed"),
                "{label} {command} must not print its error to stderr twice: {}",
                run.stderr
            );

            let log = std::fs::read_to_string(dir.join("remem.log")).unwrap_or_default();
            let recorded = log.lines().any(|line| {
                line.contains(&format!("[ERROR] [{command}]")) && line.contains("SQLCipher key")
            });
            assert!(
                recorded,
                "{label} {command} exited {:?} but remem.log does not say why:\n{log}",
                run.status.code()
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
