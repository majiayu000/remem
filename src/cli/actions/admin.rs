use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rusqlite::{Connection, DatabaseName};
use std::path::{Path, PathBuf};

use crate::cli::types::AdminAction;

pub(in crate::cli) fn run_admin(action: AdminAction) -> Result<()> {
    match action {
        AdminAction::Backup { output } => run_backup(output),
        AdminAction::ResetSchema {
            confirm_destructive,
        } => run_reset_schema(confirm_destructive),
    }
}

fn run_reset_schema(confirm_destructive: bool) -> Result<()> {
    if !confirm_destructive {
        anyhow::bail!(
            "reset-schema destroys the schema database at {}.\n\
             Re-run with --confirm-destructive to proceed.",
            crate::db::schema::default_path().display()
        );
    }
    let path = crate::db::schema::default_path();
    crate::db::schema::reset_at(&path)
        .with_context(|| format!("reset schema db at {}", path.display()))?;
    println!("Reset schema database at: {}", path.display());
    Ok(())
}

fn run_backup(output: Option<PathBuf>) -> Result<()> {
    let src_path = crate::db::db_path();
    if !src_path.exists() {
        anyhow::bail!(
            "remem database not found at {}. Nothing to back up.",
            src_path.display()
        );
    }
    let dst_path = output.unwrap_or_else(|| default_backup_path(Local::now()));
    backup_db(&src_path, &dst_path)?;
    println!("Backed up remem database to: {}", dst_path.display());
    Ok(())
}

/// `<data_dir>/backups/remem-backup-YYYYMMDD-HHMMSS.sqlite` for the given moment.
/// Pure path construction — no IO — so tests can pin the timestamp.
pub fn default_backup_path(now: DateTime<Local>) -> PathBuf {
    let timestamp = now.format("%Y%m%d-%H%M%S").to_string();
    crate::db::data_dir()
        .join("backups")
        .join(format!("remem-backup-{timestamp}.sqlite"))
}

/// Copy the SQLite file at `src_path` to `dst_path` via the SQLite Online
/// Backup API. Byte-for-byte (so an encrypted source produces an encrypted
/// backup readable with the same key) and consistent across WAL.
pub fn backup_db(src_path: &Path, dst_path: &Path) -> Result<()> {
    if !src_path.exists() {
        anyhow::bail!("source database not found at {}", src_path.display());
    }
    if let Some(parent) = dst_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create backup dir {}", parent.display()))?;
    }
    let mut src = Connection::open(src_path)
        .with_context(|| format!("open source db {}", src_path.display()))?;
    let has_cipher_key = crate::db::apply_cipher_key_if_available(&src)
        .with_context(|| format!("unlock source db {}", src_path.display()))?;
    if has_cipher_key {
        if crate::db::can_read_schema(&src) {
            backup_encrypted_db(&src, dst_path)?;
            return Ok(());
        }
        drop(src);
        src = Connection::open(src_path)
            .with_context(|| format!("reopen unencrypted source db {}", src_path.display()))?;
    }
    src.backup(DatabaseName::Main, dst_path, None)
        .with_context(|| format!("write backup to {}", dst_path.display()))?;
    Ok(())
}

fn backup_encrypted_db(src: &Connection, dst_path: &Path) -> Result<()> {
    if dst_path.exists() {
        anyhow::bail!(
            "backup destination already exists at {}",
            dst_path.display()
        );
    }
    let dst = dst_path.to_str().ok_or_else(|| {
        anyhow::anyhow!(
            "backup destination path is not valid UTF-8: {}",
            dst_path.display()
        )
    })?;
    if dst.contains('\0') {
        anyhow::bail!(
            "backup destination path contains a NUL byte: {}",
            dst_path.display()
        );
    }
    let dst_lit = dst.replace('\'', "''");
    src.execute(&format!("VACUUM INTO '{dst_lit}'"), [])
        .with_context(|| format!("write encrypted backup to {}", dst_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path, ScopedTestDataDir};
    use chrono::TimeZone;

    fn unique_temp_path() -> PathBuf {
        unique_temp_db_path("admin")
    }

    fn cleanup_paths(paths: &[&Path]) {
        for p in paths {
            cleanup_temp_db_files(p);
        }
    }

    fn create_encrypted_test_db(path: &Path, key: &str) {
        let conn = Connection::open(path).expect("open encrypted test db");
        conn.pragma_update(None, "key", key)
            .expect("apply test key");
        conn.execute_batch(
            "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); \
             INSERT INTO t(id, v) VALUES (42, 'hello-encrypted');",
        )
        .expect("seed encrypted test db");
    }

    #[test]
    fn default_backup_path_uses_timestamp() {
        let dt = Local.with_ymd_and_hms(2026, 5, 8, 14, 30, 45).unwrap();
        let path = default_backup_path(dt);
        let s = path.to_string_lossy();
        assert!(s.contains("backups"), "got {s}");
        assert!(s.contains("remem-backup-20260508-143045.sqlite"), "got {s}");
    }

    #[test]
    fn backup_copies_rows_to_target() {
        let src = unique_temp_path();
        let dst = unique_temp_path();
        {
            let conn = Connection::open(&src).unwrap();
            conn.execute_batch(
                "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); \
                 INSERT INTO t(id, v) VALUES (42, 'hello');",
            )
            .unwrap();
        }
        backup_db(&src, &dst).expect("backup");
        let restored = Connection::open(&dst).unwrap();
        let v: String = restored
            .query_row("SELECT v FROM t WHERE id = 42", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, "hello");
        cleanup_paths(&[&src, &dst]);
    }

    #[test]
    fn backup_copies_encrypted_source_with_cipher_key() {
        let test_dir = ScopedTestDataDir::new("admin-encrypted-backup");
        std::fs::create_dir_all(&test_dir.path).unwrap();
        let key = "backup-test-key";
        std::fs::write(test_dir.path.join(".key"), key).unwrap();
        let src = test_dir.db_path();
        let dst = test_dir.path.join("backup.sqlite");
        create_encrypted_test_db(&src, key);

        backup_db(&src, &dst).expect("encrypted backup");

        let restored = Connection::open(&dst).unwrap();
        restored
            .pragma_update(None, "key", key)
            .expect("apply backup key");
        let v: String = restored
            .query_row("SELECT v FROM t WHERE id = 42", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, "hello-encrypted");
    }

    #[test]
    fn backup_returns_error_when_source_missing() {
        let src = unique_temp_path();
        let dst = unique_temp_path();
        let err = backup_db(&src, &dst).unwrap_err().to_string();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn reset_schema_without_confirmation_returns_error() {
        let action = AdminAction::ResetSchema {
            confirm_destructive: false,
        };
        let err = run_admin(action).unwrap_err().to_string();
        assert!(err.contains("--confirm-destructive"), "got: {err}");
    }

    #[test]
    fn backup_creates_parent_directory() {
        let src = unique_temp_path();
        let dst = std::env::temp_dir()
            .join(format!("remem-admin-nested-{}", std::process::id()))
            .join("nested")
            .join("backup.sqlite");
        {
            let conn = Connection::open(&src).unwrap();
            conn.execute_batch("CREATE TABLE t(id INTEGER);").unwrap();
        }
        backup_db(&src, &dst).expect("backup with new dirs");
        assert!(dst.exists());
        cleanup_paths(&[&src, &dst]);
        if let Some(parent) = dst.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}
