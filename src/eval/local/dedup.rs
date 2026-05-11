use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use rusqlite::Connection;

use super::types::DedupReport;

pub(super) fn check_dedup(conn: &Connection) -> Result<DedupReport> {
    let mut stmt =
        conn.prepare("SELECT id, title, content FROM memories WHERE status = 'active'")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut hash_groups: HashMap<u64, Vec<(i64, String)>> = HashMap::new();
    for row in rows {
        let (id, title, content) = row?;
        let normalized: String = content
            .to_lowercase()
            .chars()
            .filter(|ch| ch.is_alphanumeric() || *ch == ' ')
            .take(200)
            .collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        normalized.hash(&mut hasher);
        hash_groups
            .entry(hasher.finish())
            .or_default()
            .push((id, title));
    }

    let mut duplicate_count = 0;
    let mut duplicate_groups = 0;
    let mut worst_groups = Vec::new();

    for entries in hash_groups.values() {
        if entries.len() > 1 {
            let count = entries.len() as i64;
            duplicate_groups += 1;
            duplicate_count += count - 1;
            worst_groups.push((entries[0].1.chars().take(60).collect(), count));
        }
    }

    worst_groups.sort_by_key(|right| std::cmp::Reverse(right.1));
    worst_groups.truncate(5);

    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let duplicate_rate = if total > 0 {
        duplicate_count as f64 / total as f64
    } else {
        0.0
    };

    Ok(DedupReport {
        duplicate_groups,
        duplicate_count,
        duplicate_rate,
        worst_groups,
    })
}
