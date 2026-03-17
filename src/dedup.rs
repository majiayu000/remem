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
        // Check if hash matches
        let obs_narrative: String = conn.query_row(
            "SELECT narrative FROM observations WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let obs_hash = format!("{:x}", crate::db::deterministic_hash(obs_narrative.as_bytes()));
        if obs_hash == content_hash {
            candidates.push(id);
        }
    }

    Ok(candidates)
}

/// Increment access count for duplicate observations.
pub fn mark_duplicate_accessed(conn: &Connection, obs_ids: &[i64]) -> Result<()> {
    if obs_ids.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    let placeholders: Vec<String> = (2..=obs_ids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations
         SET last_accessed_epoch = ?1
         WHERE id IN ({})",
        placeholders.join(", ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(now));
    for id in obs_ids {
        param_values.push(Box::new(*id));
    }
    let refs = crate::db::to_sql_refs(&param_values);
    stmt.execute(refs.as_slice())?;

    Ok(())
}

/// Check if new observation is a duplicate using three-layer funnel.
/// Returns Some(duplicate_id) if duplicate found, None if unique.
pub fn check_duplicate(
    conn: &Connection,
    project: &str,
    narrative: &str,
    _embedding: Option<&[f32]>,
) -> Result<Option<i64>> {
    // Layer 1: Hash deduplication (15-minute window)
    let content_hash = format!("{:x}", crate::db::deterministic_hash(narrative.as_bytes()));
    let hash_window_secs = 15 * 60; // 15 minutes

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
    // if let Some(emb) = embedding {
    //     let similar = crate::vector::find_similar_observations(conn, emb, 0.95, 10)?;
    //     if !similar.is_empty() {
    //         // Layer 3: LLM judgment for final decision
    //         // TODO: Call LLM to determine if truly duplicate
    //         return Ok(Some(similar[0]));
    //     }
    // }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_dedup_finds_exact_match() -> Result<()> {
        let mut conn = rusqlite::Connection::open_in_memory()?;

        // Initialize schema properly
        conn.execute_batch(
            "CREATE TABLE observations (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                type TEXT NOT NULL,
                title TEXT,
                subtitle TEXT,
                narrative TEXT,
                facts TEXT,
                concepts TEXT,
                files_read TEXT,
                files_modified TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0,
                status TEXT DEFAULT 'active',
                last_accessed_epoch INTEGER
            );

            CREATE TABLE sdk_sessions (
                id INTEGER PRIMARY KEY,
                content_session_id TEXT UNIQUE NOT NULL,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                user_prompt TEXT,
                started_at TEXT,
                started_at_epoch INTEGER,
                status TEXT DEFAULT 'active',
                prompt_counter INTEGER DEFAULT 1
            )",
        )?;

        // Insert observation
        let narrative = "Fixed authentication bug in login flow";
        let now = chrono::Utc::now();
        conn.execute(
            "INSERT INTO observations \
             (memory_session_id, project, type, title, narrative, created_at, created_at_epoch, discovery_tokens, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "mem-test",
                "test-project",
                "bugfix",
                "Auth fix",
                narrative,
                now.to_rfc3339(),
                now.timestamp(),
                100,
                "active"
            ],
        )?;

        // Check for duplicate
        let content_hash = format!("{:x}", crate::db::deterministic_hash(narrative.as_bytes()));
        let dups = find_hash_duplicates(&conn, "test-project", &content_hash, 900)?;

        assert_eq!(dups.len(), 1);
        Ok(())
    }
}
