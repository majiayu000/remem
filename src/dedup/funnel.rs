use anyhow::Result;
use rusqlite::Connection;

use crate::dedup::{find_hash_duplicates, mark_duplicate_accessed};

/// Check if new observation is a duplicate using three-layer funnel.
/// Returns Some(duplicate_id) if duplicate found, None if unique.
pub fn check_duplicate(
    conn: &Connection,
    project: &str,
    narrative: &str,
    _embedding: Option<&[f32]>,
) -> Result<Option<i64>> {
    let content_hash = format!("{:x}", crate::db::deterministic_hash(narrative.as_bytes()));
    let hash_window_secs = 15 * 60;

    let hash_dups = find_hash_duplicates(conn, project, &content_hash, hash_window_secs)?;
    if !hash_dups.is_empty() {
        crate::log::info(
            "dedup",
            &format!("hash duplicate found: {} matches", hash_dups.len()),
        );
        mark_duplicate_accessed(conn, &hash_dups)?;
        return Ok(Some(hash_dups[0]));
    }

    // Layer 2: Vector deduplication (cosine similarity > 0.95)
    // TODO: Implement when vector search is ready
    // Layer 3: LLM judgment

    Ok(None)
}
