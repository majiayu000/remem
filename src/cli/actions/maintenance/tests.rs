use serde_json::Value;

use super::*;
use crate::db;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory::governance::{GovernMemoryResult, GovernedMemory};
use anyhow::Result;

#[test]
fn parse_governance_id_text_accepts_commas_and_whitespace() -> Result<()> {
    let ids = parse_governance_id_text("1, 2\n3\t4")?;
    assert_eq!(ids, vec![1, 2, 3, 4]);
    Ok(())
}

#[test]
fn run_encrypt_initializes_missing_database() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("encrypt-empty");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");

    run_encrypt(false)?;

    assert!(test_dir.path.join(".key").exists());
    let saved_key = std::fs::read_to_string(test_dir.path.join(".key"))?;
    assert!(saved_key.starts_with("v2:"), "got: {saved_key}");
    assert!(test_dir.db_path().exists());
    let header = std::fs::read(test_dir.db_path())?;
    assert_ne!(&header[..16], b"SQLite format 3\0");

    let conn = rusqlite::Connection::open(test_dir.db_path())?;
    crate::db::apply_cipher_key_if_available(&conn)?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))?;
    assert!(count > 0);
    Ok(())
}

#[test]
fn run_encrypt_removes_plaintext_backup_for_existing_database() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("encrypt-existing-no-plaintext-backup");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
        conn.execute("INSERT INTO t (v) VALUES ('kept')", [])?;
    }

    run_encrypt(false)?;

    assert!(test_dir.path.join(".key").exists());
    assert!(
        !test_dir.path.join("remem.db.bak").exists(),
        "successful encryption must not leave a plaintext backup"
    );
    assert_no_plaintext_sqlite_files(&test_dir.path)?;
    let conn = rusqlite::Connection::open(test_dir.db_path())?;
    crate::db::apply_cipher_key_if_available(&conn)?;
    let value: String = conn.query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))?;
    assert_eq!(value, "kept");
    Ok(())
}

#[test]
fn run_encrypt_rolls_back_generated_key_when_preflight_fails() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("encrypt-existing-preflight-fail");
    std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
    std::env::remove_var("REMEM_CIPHER_KEY");
    std::fs::create_dir_all(&test_dir.path)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
        conn.execute("INSERT INTO t (v) VALUES ('plaintext')", [])?;
    }
    let backup_path = test_dir.path.join("remem.db.bak");
    std::fs::write(&backup_path, b"pre-existing backup sentinel")?;

    let error = run_encrypt(false).expect_err("pre-existing backup must block encryption");

    let message = format!("{error:#}");
    assert!(
        message.contains("temporary plaintext migration backup already exists"),
        "got: {message}"
    );
    assert!(
        !test_dir.path.join(".key").exists(),
        "failed preflight must remove the generated key file"
    );
    assert!(
        backup_path.exists(),
        "rollback must not remove a pre-existing backup"
    );
    let header = std::fs::read(test_dir.db_path())?;
    assert_eq!(&header[..16], b"SQLite format 3\0");
    Ok(())
}

#[test]
fn run_encrypt_rekey_raw_migrates_legacy_hex_key() -> Result<()> {
    let test_dir = ScopedTestDataDir::new("encrypt-rekey-raw");
    std::env::remove_var("REMEM_CIPHER_KEY");
    let legacy_hex = "2".repeat(64);
    std::fs::create_dir_all(&test_dir.path)?;
    std::fs::write(test_dir.path.join(".key"), &legacy_hex)?;
    {
        let conn = rusqlite::Connection::open(test_dir.db_path())?;
        conn.pragma_update(None, "key", &legacy_hex)?;
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)", [])?;
        conn.execute("INSERT INTO t (v) VALUES ('migrated')", [])?;
    }

    run_encrypt(true)?;

    let saved_key = std::fs::read_to_string(test_dir.path.join(".key"))?;
    assert_eq!(saved_key, format!("v2:{legacy_hex}"));
    let key_backup = std::fs::read_to_string(test_dir.path.join(".key.bak"))?;
    assert_eq!(key_backup, legacy_hex);
    let backup_dir = test_dir.path.join("backups");
    let backup_count = std::fs::read_dir(&backup_dir)?.count();
    assert_eq!(backup_count, 1, "expected one DB backup in {backup_dir:?}");

    let raw_conn = rusqlite::Connection::open(test_dir.db_path())?;
    crate::db::configure_cipher(
        &raw_conn,
        Some(&crate::db::CipherKey::Raw(legacy_hex.clone())),
    )?;
    let value: String = raw_conn.query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))?;
    assert_eq!(value, "migrated");

    let legacy_conn = rusqlite::Connection::open(test_dir.db_path())?;
    legacy_conn.pragma_update(None, "key", &legacy_hex)?;
    assert!(
        !crate::db::can_read_schema(&legacy_conn),
        "legacy passphrase path must no longer unlock the DB"
    );
    Ok(())
}

fn assert_no_plaintext_sqlite_files(dir: &std::path::Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let bytes = std::fs::read(entry.path())?;
        if bytes.len() < 16 {
            continue;
        }
        assert_ne!(
            &bytes[..16],
            b"SQLite format 3\0",
            "{} must not be a plaintext SQLite database",
            entry.path().display()
        );
    }
    Ok(())
}

#[test]
fn parse_governance_id_text_rejects_invalid_ids() {
    let err = parse_governance_id_text("1 nope 2").expect_err("invalid id should fail");
    assert!(err.to_string().contains("invalid memory id"));
}

#[test]
fn collect_governance_ids_reads_file_sources() -> Result<()> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "remem-governance-ids-{}-{}.txt",
        std::process::id(),
        nanos
    ));
    std::fs::write(&path, "5\n6,7")?;
    let ids = collect_governance_ids(&[4], Some(&path), false)?;
    std::fs::remove_file(&path)?;
    assert_eq!(ids, vec![4, 5, 6, 7]);
    Ok(())
}

#[test]
fn cli_governance_json_result_is_machine_parseable() -> std::result::Result<(), serde_json::Error> {
    let result = GovernMemoryResult {
        dry_run: true,
        action: "stale".to_string(),
        reason: Some("stale fact".to_string()),
        affected: vec![GovernedMemory {
            id: 7,
            title: "Old memory".to_string(),
            previous_status: "active".to_string(),
            new_status: "stale".to_string(),
        }],
    };

    let text = serde_json::to_string(&result)?;
    let parsed: Value = serde_json::from_str(&text)?;

    assert_eq!(parsed["dry_run"], true);
    assert_eq!(parsed["action"], "stale");
    assert_eq!(parsed["affected"][0]["new_status"], "stale");
    Ok(())
}

#[test]
fn cleanup_report_json_exposes_dry_run_plan_counts() -> std::result::Result<(), serde_json::Error> {
    let report = CleanupReport {
        dry_run: true,
        retention_days: CleanupRetentionDays {
            old_events: 30,
            compressed_source_observations: 90,
            stale_memories: 180,
            archived_failures: 90,
            workstream_auto_pause: 14,
            workstream_auto_abandon: 30,
        },
        plan: CleanupPlan {
            expired_memories_to_stale: 1,
            inactive_workstreams_to_pause: 2,
            long_paused_workstreams_to_abandon: 3,
            old_events_to_delete: 4,
            compressed_source_observations_to_delete: 5,
            stale_memories_to_archive: 6,
            archived_failures_to_purge: db::ArchivedFailurePurgePlan {
                pending_observations: 7,
                extraction_tasks: 8,
                extraction_replay_ranges: 9,
                jobs: 10,
            },
        },
        applied: None,
    };

    let parsed = serde_json::to_value(report)?;

    assert_eq!(parsed["dry_run"], true);
    assert_eq!(parsed["applied"], Value::Null);
    assert_eq!(parsed["plan"]["old_events_to_delete"], 4);
    assert_eq!(
        parsed["plan"]["archived_failures_to_purge"]["extraction_tasks"],
        8
    );
    assert_eq!(
        parsed["plan"]["compressed_source_observations_to_delete"],
        5
    );
    Ok(())
}
