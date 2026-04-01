use anyhow::Result;
use rusqlite::Connection;

use super::types::SelfRetrievalReport;

pub(super) fn check_self_retrieval(conn: &Connection) -> Result<SelfRetrievalReport> {
    let mut stmt = conn.prepare(
        "SELECT id, title, project FROM memories
         WHERE status = 'active' AND LENGTH(title) > 20
         ORDER BY updated_at_epoch DESC LIMIT 20",
    )?;
    let recent: Vec<(i64, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .flatten()
        .collect();

    let mut found = 0;
    let total_tested = recent.len();
    for (id, title, project) in &recent {
        let words: Vec<&str> = title
            .split_whitespace()
            .filter(|word| word.len() > 3 && !word.starts_with('—') && !word.starts_with('['))
            .take(3)
            .collect();
        if words.is_empty() {
            continue;
        }
        let query = words.join(" ");
        let results = crate::search::search(conn, Some(&query), Some(project), None, 20, 0, true)?;
        if results.iter().any(|memory| memory.id == *id) {
            found += 1;
        }
    }

    let retrieval_rate = if total_tested > 0 {
        found as f64 / total_tested as f64
    } else {
        0.0
    };
    Ok(SelfRetrievalReport {
        total_tested,
        found,
        retrieval_rate,
    })
}
