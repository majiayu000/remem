use serde_json::json;

use anyhow::Context;
use rusqlite::{params, Connection};

use super::config::{
    build_hooks, remove_remem_hooks, remove_remem_mcp, repair_hooks_json, HookStrategy,
};
use super::runtime::ensure_runtime_store_ready;
use crate::db::test_support::ScopedTestDataDir;

fn seed_plaintext_runtime_db_through(
    test_dir: &ScopedTestDataDir,
    max_migration_version: i64,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&test_dir.path)?;
    let conn = Connection::open(test_dir.db_path())?;
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
         );",
    )?;

    for migration in crate::migrate::MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= max_migration_version)
    {
        conn.execute_batch(migration.sql).with_context(|| {
            format!(
                "seed migration v{:03}_{}",
                migration.version, migration.name
            )
        })?;
        conn.execute(
            "INSERT OR IGNORE INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, ?3)",
            params![migration.version, migration.name, 1_700_000_000_i64],
        )?;
    }

    conn.execute_batch(&format!(
        "PRAGMA user_version = {};",
        12 + max_migration_version
    ))?;
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_initializes_encrypted_db_for_status() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-store");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    test_dir.remove_db_files();

    let ready = ensure_runtime_store_ready()?;

    assert!(ready.created_key);
    assert!(!ready.encrypted_existing_db);
    assert_eq!(
        ready.schema_version,
        crate::migrate::latest_schema_version()
    );
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
fn ensure_runtime_store_ready_migrates_existing_db_before_hooks() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-migrates-db");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    let latest = crate::migrate::latest_schema_version();
    seed_plaintext_runtime_db_through(&test_dir, latest - 1)?;

    let ready = ensure_runtime_store_ready()?;

    assert!(ready.created_key);
    assert!(ready.encrypted_existing_db);
    assert_eq!(ready.schema_version, latest);
    let conn = crate::db::open_db_for_hook()?;
    let latest_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM _schema_migrations WHERE version = ?1",
        [latest],
        |row| row.get(0),
    )?;
    assert_eq!(latest_rows, 1);
    let prompt_ref_column: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('graph_candidates')
         WHERE name = 'prompt_memory_ref_ids'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(prompt_ref_column, 1);
    Ok(())
}

#[test]
fn install_dry_run_does_not_initialize_runtime_store() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-dry-run-no-db");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    test_dir.remove_db_files();

    super::runtime::install(super::InstallTarget::Codex, true, false, false)?;

    assert!(
        !test_dir.path.join(".key").exists(),
        "dry-run install must not create a key file"
    );
    assert!(
        !test_dir.db_path().exists(),
        "dry-run install must not create or migrate the database"
    );
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
    assert_eq!(
        ready.schema_version,
        crate::migrate::latest_schema_version()
    );
    assert!(
        !test_dir.path.join("remem.db.bak").exists(),
        "successful automatic encryption must not leave a plaintext backup"
    );
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
fn ensure_runtime_store_ready_refuses_to_overwrite_existing_backup() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-existing-backup");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.execute("CREATE TABLE existing_probe(id INTEGER PRIMARY KEY)", [])?;
    }
    let backup_path = test_dir.db_path().with_extension("db.bak");
    std::fs::write(&backup_path, b"existing backup")?;

    let err = ensure_runtime_store_ready()
        .expect_err("install must not overwrite an existing database backup");

    let message = err.to_string();
    assert!(message.contains("would be overwritten"), "got: {message}");
    assert!(
        !test_dir.path.join(".key").exists(),
        "backup preflight failure must not create a key file"
    );
    assert_eq!(std::fs::read(&backup_path)?, b"existing backup");
    let header = std::fs::read(test_dir.db_path())?;
    assert_eq!(&header[..16], b"SQLite format 3\0");
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_rolls_back_key_when_encryption_fails() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-existing-db-encrypt-fail");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.execute("CREATE TABLE existing_probe(id INTEGER PRIMARY KEY)", [])?;
    }
    let encrypted_temp_path = test_dir.db_path().with_extension("db.enc");
    std::fs::create_dir(&encrypted_temp_path)?;

    let err = ensure_runtime_store_ready()
        .expect_err("install must fail when the encrypted temp database path is blocked");

    let message = format!("{err:#}");
    assert!(
        message.contains("encrypt existing remem database"),
        "got: {message}"
    );
    assert!(
        !test_dir.path.join(".key").exists(),
        "failed encryption must remove the generated key file"
    );
    assert!(
        test_dir.db_path().exists(),
        "failed encryption must leave the source database in place"
    );
    let header = std::fs::read(test_dir.db_path())?;
    assert_eq!(&header[..16], b"SQLite format 3\0");
    assert!(
        encrypted_temp_path.is_dir(),
        "pre-existing temp path must not be removed by rollback"
    );
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_keeps_plaintext_db_when_sidecar_cleanup_fails() -> anyhow::Result<()>
{
    let test_dir = ScopedTestDataDir::new("install-runtime-sidecar-cleanup-fail");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.execute("CREATE TABLE existing_probe(id INTEGER PRIMARY KEY)", [])?;
    }
    let sidecar_path = test_dir.path.join("remem.db-shm");
    std::fs::create_dir(&sidecar_path)?;

    let err = ensure_runtime_store_ready()
        .expect_err("install must fail before swapping when sidecar cleanup is unsafe");

    let message = format!("{err:#}");
    assert!(
        message.contains("non-file SQLite sidecar"),
        "got: {message}"
    );
    assert!(
        !test_dir.path.join(".key").exists(),
        "failed pre-swap encryption must remove the generated key file"
    );
    assert!(
        !test_dir.db_path().with_extension("db.bak").exists(),
        "pre-swap failure must not leave a plaintext backup"
    );
    assert!(
        !test_dir.db_path().with_extension("db.enc").exists(),
        "pre-swap failure rollback must remove the encrypted temp file"
    );
    assert!(sidecar_path.is_dir(), "unsafe sidecar must not be removed");
    let header = std::fs::read(test_dir.db_path())?;
    assert_eq!(&header[..16], b"SQLite format 3\0");
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
fn ensure_runtime_store_ready_rejects_env_key_mismatch_with_persisted_key() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-env-key-mismatch");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), format!("v2:{}", "7".repeat(64)))?;
    std::env::set_var("REMEM_CIPHER_KEY", format!("v2:{}", "8".repeat(64)));
    test_dir.remove_db_files();

    let result = ensure_runtime_store_ready();
    std::env::remove_var("REMEM_CIPHER_KEY");
    let err = result.expect_err("install must not initialize DB with a key that differs from .key");

    let message = err.to_string();
    assert!(
        message.contains("does not match existing SQLCipher key file"),
        "got: {message}"
    );
    assert!(
        !test_dir.db_path().exists(),
        "mismatched env key must not create a database"
    );
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_treats_empty_env_key_as_unset() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-empty-env-key");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), format!("v2:{}", "7".repeat(64)))?;
    std::env::set_var("REMEM_CIPHER_KEY", "");
    test_dir.remove_db_files();

    let result = ensure_runtime_store_ready();
    std::env::remove_var("REMEM_CIPHER_KEY");
    let ready = result?;

    assert!(!ready.created_key);
    assert!(test_dir.db_path().exists());
    let conn = crate::db::open_db_read_only()?;
    let stats = crate::db::query_system_stats(&conn)?;
    assert_eq!(stats.active_memories, 0);
    Ok(())
}

#[test]
fn ensure_runtime_store_ready_allows_empty_env_key_fresh() -> anyhow::Result<()> {
    let test_dir = ScopedTestDataDir::new("install-runtime-empty-env-key-fresh");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::set_var("REMEM_CIPHER_KEY", "");
    test_dir.remove_db_files();

    let ready = ensure_runtime_store_ready()?;

    assert!(ready.created_key);
    assert!(test_dir.path.join(".key").exists());
    assert!(test_dir.db_path().exists());
    let header = std::fs::read(test_dir.db_path())?;
    assert_ne!(&header[..16], b"SQLite format 3\0");
    Ok(())
}

#[test]
fn build_hooks_contains_expected_claude_commands() {
    let hooks = build_hooks("/tmp/remem", HookStrategy::ClaudeCode);
    assert_eq!(
        hooks["SessionStart"][0]["hooks"][0]["command"],
        "/tmp/remem context --host claude-code"
    );
    assert_eq!(
        hooks["SessionStart"][0]["matcher"],
        "startup|resume|clear|compact"
    );
    assert_eq!(hooks["SessionStart"][0]["hooks"][0]["timeout"], 15);
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
        "Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task"
    );
    assert_eq!(hooks["PostToolUse"][0]["hooks"][0]["timeout"], 120);
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
fn repair_hooks_json_preserves_third_party_and_is_idempotent() -> anyhow::Result<()> {
    let path = temp_json_path("repair-claude-hooks");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "remem": {"command": "/legacy/remem", "args": ["mcp"]}
            },
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup|clear|compact",
                    "hooks": [
                        {"type": "command", "command": "/old/remem context", "timeout": 15000},
                        {"type": "command", "command": "/opt/remem-helper prepare", "timeout": 7}
                    ]
                }],
                "PostToolUse": [{
                    "matcher": "Write|Edit|NotebookEdit|Bash",
                    "hooks": [{"type": "command", "command": "/old/remem", "args": ["observe", "--host", "claude-code"], "timeout": 120000}]
                }],
                "Stop": [{
                    "hooks": [{"type": "command", "command": "/old/remem summarize", "timeout": 120000}]
                }]
            }
        }))?,
    )?;

    let first = repair_hooks_json(&path, "/new/remem", HookStrategy::ClaudeCode)?;
    let second = repair_hooks_json(&path, "/new/remem", HookStrategy::ClaudeCode)?;
    let repaired: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;

    assert!(first.is_healthy());
    assert!(second.is_healthy());
    assert_eq!(count_command_prefix(&repaired, "/new/remem"), 5);
    assert_eq!(
        repaired["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "/opt/remem-helper prepare"
    );
    assert_eq!(repaired["mcpServers"]["remem"]["command"], "/legacy/remem");
    assert_eq!(
        repaired["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["matcher"] == "startup|resume|clear|compact")
            .unwrap()["hooks"][0]["timeout"],
        15
    );
    assert_eq!(
        repaired["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["matcher"] == "Write|Edit|NotebookEdit|Bash|Grep|Glob|Agent|Task")
            .unwrap()["hooks"][0]["timeout"],
        120
    );
    let _ = std::fs::remove_file(path);
    Ok(())
}

fn temp_json_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}.json",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn count_command_prefix(doc: &serde_json::Value, prefix: &str) -> usize {
    doc.get("hooks")
        .and_then(|hooks| hooks.as_object())
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(|entries| entries.as_array())
        .flatten()
        .filter_map(|entry| entry.get("hooks").and_then(|hooks| hooks.as_array()))
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(|command| command.as_str()))
        .filter(|command| command.starts_with(prefix))
        .count()
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
