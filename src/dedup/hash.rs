use anyhow::Result;
use rusqlite::{params, Connection};

/// Hash-based deduplication: find exact duplicates within a time window.
/// Returns observation IDs that are exact duplicates of the given content hash.
pub fn find_hash_duplicates(
    conn: &Connection,
    project: &str,
    content_hash: &str,
    window_secs: i64,
) -> Result<Vec<i64>> {
    let cutoff = chrono::Utc::now().timestamp() - window_secs;

    let mut stmt = conn.prepare(
        "SELECT id FROM observations
         WHERE project = ?1
           AND status = 'active'
           AND created_at_epoch > ?2
           AND narrative IS NOT NULL
           AND length(narrative) > 0",
    )?;

    let rows = stmt.query_map(params![project, cutoff], |row| row.get::<_, i64>(0))?;
    let mut candidates = Vec::new();
    for row in rows {
        let id = row?;
        let obs_narrative: String = conn.query_row(
            "SELECT narrative FROM observations WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let obs_hash = format!(
            "{:x}",
            crate::db::deterministic_hash(obs_narrative.as_bytes())
        );
        if obs_hash == content_hash {
            candidates.push(id);
        }
    }

    Ok(candidates)
}
