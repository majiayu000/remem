use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::backup::Backup;
use rusqlite::{Connection, DatabaseName, OpenFlags};
use sha2::{Digest, Sha256};

use crate::db::CipherKey;

pub(super) fn open_consistent_snapshot(source_path: &Path) -> Result<(Connection, String)> {
    let (source, source_key) = open_source(source_path)?;

    let mut snapshot = Connection::open_in_memory().context("open backup import snapshot")?;
    if let Some(key) = source_key.as_ref() {
        // SQLCipher refuses a backup from an encrypted source into an unkeyed
        // database ("backup is not supported with encrypted databases"), which
        // is what `remem admin backup` writes for an encrypted store. Keying
        // the in-memory snapshot with the source key lets the page copy run.
        // SQLCipher does not encrypt in-memory pages, so the snapshot and its
        // digest stay plaintext and nothing extra reaches disk (#1135).
        crate::db::apply_cipher_key(&snapshot, key).context("key backup import snapshot")?;
    }
    {
        let backup = Backup::new(&source, &mut snapshot).context("snapshot backup source")?;
        backup
            .run_to_completion(100, Duration::from_millis(1), None)
            .context("copy WAL-visible backup snapshot")?;
    }
    let serialized = snapshot
        .serialize(DatabaseName::Main)
        .context("serialize backup import snapshot")?;
    let sha256 = format!("{:x}", Sha256::digest(&*serialized));
    drop(serialized);
    Ok((snapshot, sha256))
}

/// Opens the backup read-only. Returns the cipher key that unlocked it, or
/// `None` when the source is plaintext (with or without a configured key).
fn open_source(source_path: &Path) -> Result<(Connection, Option<CipherKey>)> {
    let source = open_read_only(source_path)
        .with_context(|| format!("open source {}", source_path.display()))?;
    let key = crate::db::load_cipher_key()
        .with_context(|| format!("unlock source {}", source_path.display()))?;
    let Some(key) = key else {
        return Ok((source, None));
    };
    crate::db::apply_cipher_key(&source, &key)
        .with_context(|| format!("unlock source {}", source_path.display()))?;
    if crate::db::can_read_schema(&source) {
        return Ok((source, Some(key)));
    }
    drop(source);
    let source = open_read_only(source_path)
        .with_context(|| format!("reopen unencrypted source {}", source_path.display()))?;
    Ok((source, None))
}

fn open_read_only(source_path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}
