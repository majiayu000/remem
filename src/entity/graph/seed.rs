use anyhow::Result;
use rusqlite::Connection;

pub(super) fn load_seed_entity_ids(conn: &Connection, seed_memory_ids: &[i64]) -> Result<Vec<i64>> {
    let placeholders: Vec<String> = (1..=seed_memory_ids.len())
        .map(|index| format!("?{index}"))
        .collect();
    let sql = format!(
        "SELECT DISTINCT entity_id FROM memory_entities WHERE memory_id IN ({})",
        placeholders.join(", ")
    );
    let params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = seed_memory_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|value| value.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, i64>(0))?;
    crate::db_query::collect_rows(rows)
}
