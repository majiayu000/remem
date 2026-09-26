use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "remem-shared-roots-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(path.join("home")).unwrap();
        Self(path)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_remem"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("REMEM_DATA_DIR", self.0.join("data"))
            .env("REMEM_CONFIG", self.0.join("config.toml"));
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn write(path: &Path, value: serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("{value}\n")).unwrap();
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Catches fixed home paths, cross-host override confusion and changed ingest counts.
#[test]
fn ingest_uses_both_native_root_overrides() {
    let sandbox = Sandbox::new();
    let claude = sandbox.0.join("custom-claude");
    let codex = sandbox.0.join("custom-codex");
    write(
        &claude.join("projects/project/claude-custom.jsonl"),
        serde_json::json!({"type":"user", "sessionId":"claude-custom", "timestamp":100,
            "cwd":sandbox.0, "message":{"content":"synthetic Claude message"}}),
    );
    write(
        &codex.join("sessions/codex-custom.jsonl"),
        serde_json::json!({"type":"response_item", "session_id":"codex-custom", "timestamp":101,
            "cwd":sandbox.0, "payload":{"type":"message","role":"user","content":"synthetic Codex message"}}),
    );
    write(
        &sandbox
            .0
            .join("home/.claude/projects/default/ignored.jsonl"),
        serde_json::json!({"type":"user","message":{"content":"default must not be scanned"}}),
    );
    success(&sandbox.command().arg("encrypt").output().unwrap());
    let output = sandbox
        .command()
        .args(["ingest-sessions", "--json"])
        .env("CLAUDE_CONFIG_DIR", claude)
        .env("CODEX_HOME", codex)
        .output()
        .unwrap();
    success(&output);
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        summary,
        serde_json::json!({"scanned":2,"skipped":0,"ingested_messages":2,"failed_files":0,"partial_files":0})
    );
}

// Catches silently falling back to the real/default home after a bad override.
#[test]
fn empty_override_fails_before_database_creation() {
    for key in ["CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
        let sandbox = Sandbox::new();
        let output = sandbox
            .command()
            .args(["ingest-sessions", "--json"])
            .env(key, "")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("{key} is empty")));
        assert!(!sandbox.0.join("data/remem.db").exists());
    }
}

// Catches treating inferred defaults as required or forgetting the other host remains optional.
#[test]
fn absent_defaults_and_one_empty_valid_override_are_optional_per_host() {
    for override_root in [
        None,
        Some(("CLAUDE_CONFIG_DIR", "projects")),
        Some(("CODEX_HOME", "sessions")),
    ] {
        let sandbox = Sandbox::new();
        success(&sandbox.command().arg("encrypt").output().unwrap());
        let mut command = sandbox.command();
        command.args(["ingest-sessions", "--json"]);
        if let Some((key, suffix)) = override_root {
            let root = sandbox.0.join("explicit");
            std::fs::create_dir_all(root.join(suffix)).unwrap();
            command.env(key, root);
        }
        let output = command.output().unwrap();
        success(&output);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!({"scanned":0,"skipped":0,"ingested_messages":0,"failed_files":0,"partial_files":0})
        );
    }
}

// Catches a successful empty scan on missing/mistyped overrides, and partial ingestion of healthy roots.
#[test]
fn unavailable_explicit_roots_fail_without_importing_other_hosts() {
    for (key, suffix, other_key, other_suffix) in [
        ("CLAUDE_CONFIG_DIR", "projects", "CODEX_HOME", "sessions"),
        ("CODEX_HOME", "sessions", "CLAUDE_CONFIG_DIR", "projects"),
    ] {
        let faults = [
            "missing-host",
            "missing-child",
            "host-file",
            "child-file",
            "unreadable",
        ];
        for fault in faults {
            if fault == "unreadable" && !cfg!(unix) {
                continue;
            }
            let sandbox = Sandbox::new();
            let broken = sandbox.0.join("broken");
            match fault {
                "missing-child" => std::fs::create_dir(&broken).unwrap(),
                "host-file" => std::fs::write(&broken, "file").unwrap(),
                "child-file" => {
                    std::fs::create_dir(&broken).unwrap();
                    std::fs::write(broken.join(suffix), "file").unwrap();
                }
                "unreadable" => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::create_dir_all(broken.join(suffix)).unwrap();
                        std::fs::set_permissions(
                            broken.join(suffix),
                            std::fs::Permissions::from_mode(0),
                        )
                        .unwrap();
                    }
                }
                _ => {}
            }
            let healthy = sandbox.0.join("healthy");
            write(
                &healthy.join(other_suffix).join("s.jsonl"),
                serde_json::json!({"type":"user","message":{"content":"must stay unimported"}}),
            );
            success(&sandbox.command().arg("encrypt").output().unwrap());
            let output = sandbox
                .command()
                .args(["ingest-sessions", "--json"])
                .env(key, &broken)
                .env(other_key, &healthy)
                .output()
                .unwrap();
            #[cfg(unix)]
            if fault == "unreadable" {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    broken.join(suffix),
                    std::fs::Permissions::from_mode(0o700),
                )
                .unwrap();
            }
            assert!(!output.status.success(), "{key} {fault}");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
                serde_json::json!({"scanned":1,"skipped":0,"ingested_messages":0,"failed_files":1,"partial_files":0})
            );
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains(broken.join(suffix).to_string_lossy().as_ref()));
            let conn = rusqlite::Connection::open(sandbox.0.join("data/remem.db")).unwrap();
            for table in ["raw_session_identities", "raw_messages", "ingest_cursors"] {
                assert_eq!(
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0,
                    "{key} {fault} {table}"
                );
            }
        }
    }
}
