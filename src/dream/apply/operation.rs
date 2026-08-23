use anyhow::{Context, Result};
use rusqlite::{params, Connection};

pub(super) fn reason(activation_id: &str) -> String {
    format!("dream consolidation applied activation={activation_id}")
}

pub(super) fn id_for_activation(
    conn: &Connection,
    memory_id: i64,
    activation_id: &str,
) -> Result<i64> {
    let reason = reason(activation_id);
    conn.query_row(
        "SELECT id FROM memory_operation_log
         WHERE source = 'dream' AND result_memory_id = ?1 AND reason = ?2",
        params![memory_id, reason],
        |row| row.get(0),
    )
    .context("dream replay is missing its activation-bound operation audit")
}
