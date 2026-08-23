use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

use super::plan::CleanupGroupApplyResult;

pub(super) fn insert(
    conn: &Connection,
    activation_id: &str,
    result: &CleanupGroupApplyResult,
) -> Result<()> {
    let response_json = serde_json::to_string(result)
        .context("serialize scope cleanup activation response receipt")?;
    let updated = conn
        .execute(
            "UPDATE memory_operation_log
             SET scope_cleanup_response_json = ?1
             WHERE id = ?2 AND activation_id IS NULL",
            params![response_json, result.operation_id],
        )
        .context("stage scope cleanup response on operation log")?;
    if updated != 1 {
        bail!(
            "failed to stage cleanup response on operation {}",
            result.operation_id
        );
    }
    conn.execute(
        "INSERT INTO memory_scope_cleanup_receipts
         (activation_id, result_memory_id, operation_id, response_json, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            activation_id,
            result.current_id,
            result.operation_id,
            response_json,
            chrono::Utc::now().timestamp(),
        ],
    )
    .context("insert scope cleanup activation response receipt")?;
    Ok(())
}

pub(super) fn load(
    conn: &Connection,
    activation_id: &str,
    expected_memory_id: i64,
) -> Result<CleanupGroupApplyResult> {
    let (memory_id, operation_id, response_json): (i64, i64, String) = conn
        .query_row(
            "SELECT result_memory_id, operation_id, response_json
             FROM memory_scope_cleanup_receipts WHERE activation_id = ?1",
            [activation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("load scope cleanup activation response receipt")?;
    let result: CleanupGroupApplyResult = serde_json::from_str(&response_json)
        .context("decode scope cleanup activation response receipt")?;
    if memory_id != expected_memory_id
        || result.current_id != memory_id
        || result.operation_id != operation_id
    {
        bail!("scope cleanup activation response receipt identity does not match");
    }
    Ok(result)
}
