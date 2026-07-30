//! Connection pragmas shared by every way remem opens the store (GH949).
//!
//! Previously each `open_configured_*` helper in [`super::core`] repeated its
//! own pragma string, so tuning drifted between read-write and read-only paths
//! and the performance pragmas were missing entirely. All connection setup now
//! funnels through [`apply_connection_pragmas`].
//!
//! `PRAGMA optimize` is deliberately not wired here. It pays off on a
//! long-lived connection at close time, but the worker opens and drops a fresh
//! connection every loop iteration and hook subcommands are one-shot, so the
//! only sensible host is the daemon's periodic maintenance section — tracked
//! separately to keep this change off that code path.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Page cache per connection, expressed in KiB. SQLite treats a negative
/// `cache_size` as "this many KiB" rather than "this many pages", which keeps
/// the footprint predictable regardless of page size.
///
/// 64 MiB rather than the ~2 MiB default because SQLCipher disables mmap: every
/// cache miss on a multi-GB store costs a `pread` plus an AES decrypt, so page
/// residency is worth far more here than on a plaintext database.
const DEFAULT_CACHE_KIB: i64 = 65_536;

/// Overrides [`DEFAULT_CACHE_KIB`]. Must parse as a positive integer when set;
/// a malformed value fails the open rather than silently reverting to the
/// default, so an operator never believes a typo took effect.
const CACHE_KIB_ENV: &str = "REMEM_SQLITE_CACHE_KIB";

/// How a connection intends to use the database. Read-only connections cannot
/// change `journal_mode` or meaningfully set `synchronous`, so they take a
/// narrower pragma set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionMode {
    ReadWrite,
    ReadOnly,
}

/// Pure so it is testable without mutating process-global environment, which
/// would race against every other test in the binary.
fn parse_cache_kib(raw: Option<&str>) -> Result<i64> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_CACHE_KIB);
    };
    let trimmed = raw.trim();
    let parsed: i64 = trimmed.parse().with_context(|| {
        format!("{CACHE_KIB_ENV} must be a positive integer of KiB, got {trimmed:?}")
    })?;
    if parsed <= 0 {
        anyhow::bail!("{CACHE_KIB_ENV} must be a positive integer of KiB, got {parsed}");
    }
    Ok(parsed)
}

fn cache_kib() -> Result<i64> {
    parse_cache_kib(std::env::var(CACHE_KIB_ENV).ok().as_deref())
}

fn pragma_batch_with(mode: ConnectionMode, cache: i64) -> String {
    let mut batch = String::new();
    if mode == ConnectionMode::ReadWrite {
        // WAL first: `synchronous=NORMAL` is only crash-safe once the journal
        // is WAL, where it costs at most the most recent commits on power loss
        // and never risks corruption.
        batch.push_str("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; ");
    }
    batch.push_str("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; ");
    batch.push_str("PRAGMA temp_store=MEMORY; ");
    batch.push_str(&format!("PRAGMA cache_size=-{cache};"));
    batch
}

pub(crate) fn pragma_batch(mode: ConnectionMode) -> Result<String> {
    Ok(pragma_batch_with(mode, cache_kib()?))
}

pub(crate) fn apply_connection_pragmas(conn: &Connection, mode: ConnectionMode) -> Result<()> {
    conn.execute_batch(&pragma_batch(mode)?)
        .context("failed to apply connection pragmas")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_cache_override_falls_back_to_default() {
        assert_eq!(parse_cache_kib(None).unwrap(), DEFAULT_CACHE_KIB);
    }

    #[test]
    fn cache_override_is_parsed_and_trimmed() {
        assert_eq!(parse_cache_kib(Some(" 512 ")).unwrap(), 512);
    }

    #[test]
    fn malformed_cache_override_fails_closed() {
        assert!(parse_cache_kib(Some("not-a-number")).is_err());
        assert!(parse_cache_kib(Some("0")).is_err());
        assert!(parse_cache_kib(Some("-1")).is_err());
    }

    #[test]
    fn read_only_batch_omits_write_only_pragmas() {
        let batch = pragma_batch_with(ConnectionMode::ReadOnly, DEFAULT_CACHE_KIB);
        assert!(!batch.contains("journal_mode"));
        assert!(!batch.contains("synchronous"));
        assert!(batch.contains("PRAGMA temp_store=MEMORY"));
        assert!(batch.contains("PRAGMA cache_size=-65536"));
        assert!(batch.contains("PRAGMA foreign_keys=ON"));
    }

    #[test]
    fn read_write_batch_sets_wal_before_synchronous() {
        let batch = pragma_batch_with(ConnectionMode::ReadWrite, DEFAULT_CACHE_KIB);
        let wal = batch.find("journal_mode=WAL").expect("WAL pragma present");
        let sync = batch
            .find("synchronous=NORMAL")
            .expect("synchronous pragma present");
        assert!(wal < sync, "synchronous=NORMAL is only safe under WAL");
    }

    #[test]
    fn pragmas_take_effect_on_a_real_connection() {
        let conn = Connection::open_in_memory().unwrap();
        apply_connection_pragmas(&conn, ConnectionMode::ReadWrite).unwrap();

        let cache: i64 = conn
            .query_row("PRAGMA cache_size", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cache, -DEFAULT_CACHE_KIB);

        // 2 == MEMORY
        let temp_store: i64 = conn
            .query_row("PRAGMA temp_store", [], |row| row.get(0))
            .unwrap();
        assert_eq!(temp_store, 2);

        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
    }
}
