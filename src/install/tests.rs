use serde_json::json;

use super::config::{build_hooks, remove_remem_hooks, remove_remem_mcp, HookStrategy};
use super::runtime::ensure_runtime_store_ready;
use crate::db::test_support::ScopedTestDataDir;

#[test]
fn ensure_runtime_store_ready_initializes_encrypted_db_for_status() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-store");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    test_dir.remove_db_files();

    let ready = ensure_runtime_store_ready()?;

    assert!(ready.created_key);
    assert!(!ready.encrypted_existing_db);
    assert_eq!(ready.key_path, test_dir.path.join(".key"));
    assert_eq!(ready.db_path, test_dir.db_path());
    let saved_key = std::fs::read_to_string(&ready.key_path)?;
    assert!(saved_key.starts_with("v2:"), "got: {saved_key}");
    let header = std::fs::read(&ready.db_path)?;
    assert_ne!(&header[..16], b"SQLite format 3\0");

    let conn = crate::db::open_db_read_only()?;
    let stats = crate::db::query_system_stats(&conn)?;
    assert_eq!(stats.active_memories, 0);
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_encrypts_existing_plaintext_db() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-existing-db");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.execute("CREATE TABLE existing_probe(id INTEGER PRIMARY KEY)", [])?;
    }

    let ready = ensure_runtime_store_ready()?;

    assert!(ready.created_key);
    assert!(ready.encrypted_existing_db);
    assert!(test_dir.path.join("remem.db.bak").exists());
    let header = std::fs::read(&ready.db_path)?;
    assert_ne!(&header[..16], b"SQLite format 3\0");
    let conn = crate::db::open_db_read_only()?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'existing_probe'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_refuses_existing_non_plaintext_db_without_key() -> anyhow::Result<()>
{
    let test_dir = ScopedTestDataDir::new("install-runtime-existing-encrypted-db");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    let raw_hex = "7".repeat(64);
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        crate::db::configure_cipher(&conn, Some(&crate::db::CipherKey::Raw(raw_hex)))?;
        conn.execute("CREATE TABLE encrypted_probe(id INTEGER PRIMARY KEY)", [])?;
    }

    let err = ensure_runtime_store_ready()
        .expect_err("install must not invent a new key for an encrypted database");

    let message = err.to_string();
    assert!(
        message.contains("does not look like plaintext SQLite"),
        "got: {message}"
    );
    assert!(
        !test_dir.path.join(".key").exists(),
        "failed install must not create a misleading key"
    );
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_refuses_env_only_key_without_key_file() {
    let test_dir = ScopedTestDataDir::new("install-runtime-env-only-key");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::set_var("REMEM_CIPHER_KEY", "7".repeat(64));
    test_dir.remove_db_files();

    let err = ensure_runtime_store_ready()
        .expect_err("install must require a persistent key file for future status");

    let message = err.to_string();
    assert!(
        message.contains("REMEM_CIPHER_KEY is set"),
        "got: {message}"
    );
    assert!(
        !test_dir.path.join(".key").exists(),
        "env-only failure must not create a key file"
    );
    assert!(
        !test_dir.db_path().exists(),
        "env-only failure must not create a database"
    );
}

#[test]
fn build_hooks_contains_expected_claude_commands() {
    let hooks = build_hooks("/tmp/remem", HookStrategy::ClaudeCode);
    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "/tmp/remem context --host claude-code"
    );
    assert_eq!(hooks["SessionStart"][0]["matcher"], "startup|clear|compact");
    assert_eq!(
        hooks["UserPromptSubmit"][0]["hooks"][0]["command"],
        "/tmp/remem session-init --host claude-code"
    );
    assert_eq!(
        hooks["PostToolUse"][0]["hooks"][0]["command"],
        "/tmp/remem observe --host claude-code"
    );
    assert_eq!(
        hooks["PostToolUse"][0]["matcher"],
        "Write|Edit|NotebookEdit|Bash|Grep|Glob|Task"
    );
    assert_eq!(
        hooks["Stop"][0]["hooks"][0]["command"],
        "/tmp/remem summarize --host claude-code"
    );
    assert_eq!(
        hooks["PreCompact"][0]["hooks"][0]["command"],
        "/tmp/remem summarize --host claude-code"
    );
}

#[test]
fn build_hooks_contains_expected_codex_commands() {
    let hooks = build_hooks("/tmp/remem", HookStrategy::Codex);
    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "/tmp/remem context --host codex-cli"
    );
    assert!(hooks["SessionStart"][0].get("matcher").is_none());
    assert!(hooks.get("UserPromptSubmit").is_none());
    assert!(hooks.get("PostToolUse").is_none());
    assert!(hooks.get("PreCompact").is_none());
    assert_eq!(
        hooks["Stop"][0]["hooks"][0]["command"],
        "/tmp/remem summarize --host codex-cli"
    );
}

#[test]
fn build_hooks_quotes_binary_paths_with_spaces() {
    let hooks = build_hooks("/tmp/remem bin/remem", HookStrategy::Codex);

    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "'/tmp/remem bin/remem' context --host codex-cli"
    );
    assert_eq!(
        hooks["Stop"][0]["hooks"][0]["command"],
        "'/tmp/remem bin/remem' summarize --host codex-cli"
    );
}

#[test]
fn build_hooks_quotes_binary_paths_with_single_quotes() {
    let hooks = build_hooks("/tmp/remem'bin/remem", HookStrategy::ClaudeCode);

    assert_eq!(
        hooks["PostToolUse"][0]["hooks"][0]["command"],
        "'/tmp/remem'\\''bin/remem' observe --host claude-code"
    );
}

#[test]
fn remove_remem_hooks_preserves_other_hooks() {
    let mut settings = json!({
        "hooks": {
            "SessionStart": [
                {"hooks": [{"command": "/tmp/remem context"}]},
                {"hooks": [{"command": "other-tool prepare"}]}
            ],
            "Stop": [
                {"hooks": [{"command": "remem summarize"}]}
            ]
        }
    });

    remove_remem_hooks(&mut settings, "/tmp/remem");

    assert_eq!(
        settings["hooks"]["SessionStart"]
            .as_array()
            .map(|arr| arr.len()),
        Some(1)
    );
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "other-tool prepare"
    );
    assert!(settings["hooks"].get("Stop").is_none());
}

#[test]
fn remove_remem_mcp_removes_named_and_command_matched_servers() {
    let mut settings = json!({
        "mcpServers": {
            "remem": {"command": "/tmp/remem", "args": ["mcp"]},
            "shadow": {"command": "/tmp/remem-alt", "args": []},
            "keep": {"command": "/usr/bin/other", "args": []}
        }
    });

    remove_remem_mcp(&mut settings, "/tmp/remem");

    assert!(settings["mcpServers"].get("remem").is_none());
    assert!(settings["mcpServers"].get("shadow").is_none());
    assert_eq!(settings["mcpServers"]["keep"]["command"], "/usr/bin/other");
}
