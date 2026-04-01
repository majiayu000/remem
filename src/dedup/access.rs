use anyhow::Result;
use rusqlite::Connection;

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
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    for id in obs_ids {
        param_values.push(Box::new(*id));
    }
    let refs = crate::db::to_sql_refs(&param_values);
    stmt.execute(refs.as_slice())?;

    Ok(())
}
