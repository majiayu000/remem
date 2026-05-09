//! `event_blobs` writer for the v2 capture path. SPEC-memory-system-v2.1
//! §4 D1 spills oversize content out of the inline `content_text` column
//! and into this table; B.1.x stores plain bytes, B.1.y will add gzip for
//! payloads above 256 KiB.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// Insert a plain-encoded blob row for `content_bytes`. If a row with the
/// same `content_hash` already exists, return the existing id without
/// re-inserting (dedupe across captured events that share content).
pub fn insert_or_get_blob(
    conn: &Connection,
    content_hash: &str,
    content_bytes: &[u8],
    now: i64,
) -> Result<i64> {
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM event_blobs WHERE content_hash = ?1",
            [content_hash],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    let original = content_bytes.len() as i64;
    conn.execute(
        "INSERT INTO event_blobs(content_hash, content_encoding, content_bytes,
            original_bytes, stored_bytes, created_at_epoch)
         VALUES (?1, 'plain', ?2, ?3, ?3, ?4)",
        params![content_hash, content_bytes, original, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Build the inline `content_text` summary for an oversize event:
/// `<prefix>\n\n... [truncated; full content N bytes in event_blobs] ...\n\n<suffix>`.
/// Both ends are truncated on UTF-8 boundaries so multi-byte characters are
/// not split across the cut.
pub fn summarize_oversize(content: &str, prefix_bytes: usize, suffix_bytes: usize) -> String {
    let total_len = content.len();
    let mut p_cut = prefix_bytes.min(total_len);
    while p_cut > 0 && !content.is_char_boundary(p_cut) {
        p_cut -= 1;
    }
    let mut s_start = total_len.saturating_sub(suffix_bytes);
    while s_start < total_len && !content.is_char_boundary(s_start) {
        s_start += 1;
    }
    let prefix = &content[..p_cut];
    let suffix = &content[s_start..];
    format!(
        "{prefix}\n\n... [truncated; full content {total_len} bytes in event_blobs] ...\n\n{suffix}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};
    use crate::v2_db::open_v2_db_at;

    fn open_v2() -> (rusqlite::Connection, std::path::PathBuf) {
        let path = unique_temp_db_path("blob");
        let conn = open_v2_db_at(&path).unwrap();
        (conn, path)
    }

    #[test]
    fn insert_blob_writes_row_with_plain_encoding() {
        let (conn, path) = open_v2();
        let bytes = b"hello world";
        let id = insert_or_get_blob(&conn, "h1", bytes, 100).unwrap();
        assert!(id > 0);
        let (encoding, original, stored): (String, i64, i64) = conn
            .query_row(
                "SELECT content_encoding, original_bytes, stored_bytes FROM event_blobs WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(encoding, "plain");
        assert_eq!(original, bytes.len() as i64);
        assert_eq!(stored, original, "B.1.x stores plain — no compression");
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn duplicate_hash_dedupes_to_same_row() {
        let (conn, path) = open_v2();
        let id1 = insert_or_get_blob(&conn, "dup", b"payload", 100).unwrap();
        let id2 = insert_or_get_blob(&conn, "dup", b"payload", 200).unwrap();
        assert_eq!(id1, id2);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM event_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn summarize_keeps_prefix_and_suffix_on_byte_boundary() {
        let content = "AAAA".repeat(1000); // 4000 bytes ASCII
        let summary = summarize_oversize(&content, 16, 16);
        assert!(summary.starts_with("AAAAAAAAAAAAAAAA"), "16-byte prefix");
        assert!(summary.ends_with("AAAAAAAAAAAAAAAA"), "16-byte suffix");
        assert!(summary.contains("4000 bytes"), "size marker present: {summary}");
    }

    #[test]
    fn summarize_respects_utf8_boundaries() {
        // 中 is 3 bytes; place one at the boundary so a naive cut would split it.
        let mut s = "a".repeat(15);
        s.push('中'); // bytes 15..18
        s.push_str(&"b".repeat(2000));
        let summary = summarize_oversize(&s, 16, 16);
        // The prefix should not include a partial multi-byte char — the cut
        // backs off to byte 15 (the boundary before '中').
        assert!(summary.starts_with(&"a".repeat(15)));
        assert!(!summary.starts_with(&format!("{}\u{0}", "a".repeat(15))));
    }

    #[test]
    fn summarize_handles_overlapping_prefix_suffix() {
        // Content shorter than prefix + suffix: both halves can overlap.
        let s = "abcdef";
        let summary = summarize_oversize(s, 4, 4);
        assert!(summary.contains("6 bytes"));
        // Allow either prefix == "abcd" + suffix == "cdef" or the whole string
        // appearing twice; just verify it does not panic and contains the marker.
    }
}
