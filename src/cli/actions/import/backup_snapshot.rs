use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::backup::Backup;
use rusqlite::{Connection, DatabaseName, OpenFlags};
use sha2::{Digest, Sha256};

pub(super) fn open_consistent_snapshot(source_path: &Path) -> Result<(Connection, String)> {
    let mut source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open source {}", source_path.display()))?;
    let has_cipher_key = crate::db::apply_cipher_key_if_available(&source)
        .with_context(|| format!("unlock source {}", source_path.display()))?;
    if has_cipher_key && !crate::db::can_read_schema(&source) {
        drop(source);
        source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("reopen unencrypted source {}", source_path.display()))?;
    }

    let mut snapshot = Connection::open_in_memory().context("open backup import snapshot")?;
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
