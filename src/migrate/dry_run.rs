use anyhow::{bail, ensure, Context, Result};
use rusqlite::{
    backup::Backup,
    types::{Type, ValueRef},
    Connection,
};
use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::run::run_post_migration_hook;
use super::state::{applied_versions, has_migration_table};
use super::transition::backfill_to_baseline;
use super::types::{DryRunResult, Migration, MIGRATIONS, OLD_BASELINE_VERSION};

pub(crate) fn dry_run_pending(real_conn: &Connection) -> Result<DryRunResult> {
    let raw_current_version: i64 = real_conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    let applied = infer_applied_versions(real_conn, raw_current_version)?;
    let current_version = logical_current_version(raw_current_version, &applied);

    let temp_path = match DryRunTempPath::create() {
        Ok(temp_path) => temp_path,
        Err(error) => {
            return Ok(DryRunResult {
                current_version,
                pending_count: applied_pending_count(&applied),
                error: Some(format!("database clone: {}", error)),
            });
        }
    };
    let mut test_conn = match Connection::open(temp_path.path()) {
        Ok(conn) => conn,
        Err(error) => {
            return Ok(DryRunResult {
                current_version,
                pending_count: applied_pending_count(&applied),
                error: Some(format!("database clone: {}", error)),
            });
        }
    };
    if let Some(key) = crate::db::load_cipher_key() {
        if let Err(error) = crate::db::configure_cipher(&test_conn, Some(&key)) {
            return Ok(DryRunResult {
                current_version,
                pending_count: applied_pending_count(&applied),
                error: Some(format!("database clone: {}", error)),
            });
        }
    }
    if let Err(error) = clone_database(real_conn, &mut test_conn) {
        return Ok(DryRunResult {
            current_version,
            pending_count: applied_pending_count(&applied),
            error: Some(format!("database clone: {}", error)),
        });
    }
    if raw_current_version >= OLD_BASELINE_VERSION || has_migration_table(real_conn) {
        if let Err(error) = backfill_to_baseline(&test_conn) {
            return Ok(DryRunResult {
                current_version,
                pending_count: applied_pending_count(&applied),
                error: Some(format!("baseline backfill: {}", error)),
            });
        }
    }

    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .collect();

    if pending.is_empty() {
        return Ok(DryRunResult {
            current_version,
            pending_count: 0,
            error: None,
        });
    }

    for migration in &pending {
        if let Err(error) = test_conn.execute_batch(migration.sql) {
            return Ok(DryRunResult {
                current_version,
                pending_count: pending.len(),
                error: Some(format!(
                    "v{:03}_{}: {}",
                    migration.version, migration.name, error
                )),
            });
        }
        if let Err(error) = run_post_migration_hook(&test_conn, migration.version, migration.name) {
            return Ok(DryRunResult {
                current_version,
                pending_count: pending.len(),
                error: Some(format!(
                    "v{:03}_{} post-migration hook: {}",
                    migration.version, migration.name, error
                )),
            });
        }
    }

    if let Err(error) = backfill_to_baseline(&test_conn) {
        return Ok(DryRunResult {
            current_version,
            pending_count: pending.len(),
            error: Some(format!("baseline backfill: {}", error)),
        });
    }

    Ok(DryRunResult {
        current_version,
        pending_count: pending.len(),
        error: None,
    })
}

fn applied_pending_count(applied: &[i64]) -> usize {
    MIGRATIONS
        .iter()
        .filter(|migration| !applied.contains(&migration.version))
        .count()
}

fn infer_applied_versions(conn: &Connection, current_version: i64) -> Result<Vec<i64>> {
    if has_migration_table(conn) {
        return applied_versions(conn);
    }
    if current_version >= OLD_BASELINE_VERSION {
        return Ok(vec![1]);
    }
    Ok(Vec::new())
}

fn logical_current_version(raw_current_version: i64, applied: &[i64]) -> i64 {
    let Some(latest_applied) = applied.iter().max() else {
        return raw_current_version;
    };
    raw_current_version.max(OLD_BASELINE_VERSION - 1 + latest_applied)
}

fn clone_database(src: &Connection, dst: &mut Connection) -> Result<()> {
    let page_size = query_page_size(src)?;
    ensure!(page_size > 0, "source database page_size must be positive");
    dst.execute_batch(&format!("PRAGMA page_size = {page_size}"))?;
    let backup = Backup::new(src, dst)?;
    backup.run_to_completion(100, Duration::from_millis(1), None)?;
    Ok(())
}

fn query_page_size(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA page_size", [], |row| match row.get_ref(0)? {
        ValueRef::Integer(value) => Ok(value),
        ValueRef::Text(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })?;
            text.parse::<i64>().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })
        }
        other => Err(rusqlite::Error::InvalidColumnType(
            0,
            "page_size".to_string(),
            other.data_type(),
        )),
    })
    .map_err(Into::into)
}

struct DryRunTempPath {
    path: PathBuf,
}

impl DryRunTempPath {
    fn create() -> Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let temp_dir = std::env::temp_dir();
        for _ in 0..32 {
            let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system time before unix epoch")?
                .as_nanos();
            let path = temp_dir.join(format!("remem-dry-run-{}-{nonce}-{counter}", process::id()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("create dry-run database {}", path.display()));
                }
            }
        }

        bail!("create unique dry-run database path")
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DryRunTempPath {
    fn drop(&mut self) {
        cleanup_sqlite_files(&self.path);
    }
}

fn cleanup_sqlite_files(path: &Path) {
    remove_sqlite_file(path);
    remove_sqlite_file(&sqlite_sidecar_path(path, "-wal"));
    remove_sqlite_file(&sqlite_sidecar_path(path, "-shm"));
    remove_sqlite_file(&sqlite_sidecar_path(path, "-journal"));
}

fn remove_sqlite_file(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => eprintln!(
            "failed to remove dry-run database file {}: {}",
            path.display(),
            error
        ),
    }
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};

    #[cfg(unix)]
    #[test]
    fn dry_run_temp_path_uses_owner_only_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let temp_path = DryRunTempPath::create()?;
        let mode = fs::metadata(temp_path.path())?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        Ok(())
    }

    #[test]
    fn dry_run_temp_path_uses_on_disk_database_and_cleans_up() -> Result<()> {
        let source_path = unique_temp_db_path("dry-run-source");
        let dry_run_path;
        {
            let source = Connection::open(&source_path)?;
            source.execute_batch(
                "PRAGMA page_size = 8192;
                 CREATE TABLE items (id INTEGER PRIMARY KEY);
                 INSERT INTO items DEFAULT VALUES;",
            )?;

            let temp_path = DryRunTempPath::create()?;
            dry_run_path = temp_path.path().to_path_buf();
            {
                let mut dst = Connection::open(temp_path.path())?;
                clone_database(&source, &mut dst)?;

                let count: i64 =
                    dst.query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))?;
                assert_eq!(count, 1);

                let page_size: i64 = dst.query_row("PRAGMA page_size", [], |row| row.get(0))?;
                assert_eq!(page_size, 8192);
            }

            assert!(
                dry_run_path.exists(),
                "dry-run clone should use a temporary on-disk database file"
            );
        }
        assert!(
            !dry_run_path.exists(),
            "temporary dry-run database should be removed after use"
        );
        assert!(!sqlite_sidecar_path(&dry_run_path, "-wal").exists());
        assert!(!sqlite_sidecar_path(&dry_run_path, "-shm").exists());

        cleanup_temp_db_files(&source_path);
        Ok(())
    }
}
