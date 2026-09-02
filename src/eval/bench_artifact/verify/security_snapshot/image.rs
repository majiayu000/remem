use std::fs;
use std::ptr::{self, NonNull};

use anyhow::{ensure, Context, Result};
use rusqlite::serialize::OwnedData;
use rusqlite::{ffi, Connection, DatabaseName};

use super::super::VerifyState;

pub(super) const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub(in crate::eval::bench_artifact::verify) fn validate_size(
    path: &std::path::Path,
    label: &str,
    state: &mut VerifyState,
) -> bool {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() <= MAX_BYTES => true,
        Ok(metadata) => {
            state.fail(
                label.to_string(),
                format!(
                    "security SQLite snapshot exceeds {MAX_BYTES} bytes: {}",
                    metadata.len()
                ),
            );
            false
        }
        Err(error) => {
            state.fail(
                label.to_string(),
                format!("inspect security SQLite snapshot: {error}"),
            );
            false
        }
    }
}

pub(super) fn validate_canonical(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_BYTES as usize,
        "security SQLite snapshot exceeds {MAX_BYTES} bytes"
    );
    let connection = open_consumed(bytes, false)?;
    connection.execute_batch("VACUUM")?;
    let mut canonical = connection.serialize(DatabaseName::Main)?.to_vec();
    let mut consumed = bytes.to_vec();
    normalize_vacuum_header(&mut canonical)?;
    normalize_vacuum_header(&mut consumed)?;
    let differing_offsets = consumed
        .iter()
        .zip(&canonical)
        .enumerate()
        .filter_map(|(offset, (left, right))| (left != right).then_some(offset))
        .take(16)
        .collect::<Vec<_>>();
    ensure!(
        differing_offsets.is_empty() && consumed.len() == canonical.len(),
        "snapshot bytes differ from a canonical VACUUM image at offsets {differing_offsets:?} ({} vs {} bytes)",
        consumed.len(),
        canonical.len()
    );
    Ok(())
}

pub(in crate::eval::bench_artifact::verify) fn open_read_only(bytes: &[u8]) -> Result<Connection> {
    open_consumed(bytes, true)
}

fn open_consumed(bytes: &[u8], read_only: bool) -> Result<Connection> {
    validate_length(bytes)?;
    let allocation_size = u64::try_from(bytes.len()).context("SQLite snapshot is too large")?;
    // SAFETY: SQLite allocates the destination, the copy has the exact initialized length, and
    // OwnedData immediately receives exclusive ownership and frees it with sqlite3_free.
    let owned = unsafe {
        let allocation = ffi::sqlite3_malloc64(allocation_size);
        let allocation =
            NonNull::new(allocation.cast::<u8>()).context("allocate consumed SQLite snapshot")?;
        ptr::copy_nonoverlapping(bytes.as_ptr(), allocation.as_ptr(), bytes.len());
        OwnedData::from_raw_nonnull(allocation, bytes.len())
    };
    let mut connection = Connection::open_in_memory().context("open in-memory SQLite handle")?;
    connection
        .deserialize(DatabaseName::Main, owned, read_only)
        .context("deserialize consumed SQLite snapshot")?;
    Ok(connection)
}

fn validate_length(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() >= 100, "SQLite snapshot header is incomplete");
    ensure!(
        &bytes[..16] == b"SQLite format 3\0",
        "SQLite snapshot header magic is invalid"
    );
    let raw_page_size = u16::from_be_bytes([bytes[16], bytes[17]]);
    let page_size = if raw_page_size == 1 {
        65_536
    } else {
        usize::from(raw_page_size)
    };
    ensure!(
        page_size == 65_536 || (512..=32_768).contains(&page_size) && page_size.is_power_of_two(),
        "SQLite snapshot page size is invalid"
    );
    let page_count = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]) as usize;
    ensure!(page_count > 0, "SQLite snapshot page count is zero");
    let expected_len = page_size
        .checked_mul(page_count)
        .context("SQLite snapshot image length overflows")?;
    ensure!(
        bytes.len() == expected_len,
        "SQLite snapshot length differs from header: expected {expected_len} bytes, got {}",
        bytes.len()
    );
    Ok(())
}

fn normalize_vacuum_header(bytes: &mut [u8]) -> Result<()> {
    ensure!(bytes.len() >= 100, "SQLite snapshot header is incomplete");
    // VACUUM advances the change counter, schema cookie, version-valid-for, and writer version.
    for range in [24..28, 40..44, 92..96, 96..100] {
        bytes[range].fill(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(vacuum: bool) -> Result<Vec<u8>> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE TABLE evidence(value TEXT);\
             INSERT INTO evidence VALUES('private deleted payload');\
             DELETE FROM evidence;",
        )?;
        if vacuum {
            connection.execute_batch("VACUUM")?;
        }
        Ok(connection.serialize(DatabaseName::Main)?.to_vec())
    }

    #[test]
    fn canonical_vacuum_image_is_accepted() -> Result<()> {
        validate_canonical(&image(true)?)
    }

    #[test]
    fn image_with_deleted_page_payload_is_rejected() -> Result<()> {
        assert!(validate_canonical(&image(false)?).is_err());
        Ok(())
    }
}
