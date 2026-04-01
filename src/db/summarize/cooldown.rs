use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

pub fn is_summarize_on_cooldown(
    conn: &Connection,
    project: &str,
    cooldown_secs: i64,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT last_summarize_epoch FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    );

    match result {
        Ok(last_epoch) => Ok(now - last_epoch < cooldown_secs),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn is_duplicate_message(conn: &Connection, project: &str, message_hash: &str) -> Result<bool> {
    let result: rusqlite::Result<Option<String>> = conn.query_row(
        "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    );

    match result {
        Ok(Some(prev_hash)) => Ok(prev_hash == message_hash),
        Ok(None) => Ok(false),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub fn try_acquire_summarize_lock(
    conn: &mut Connection,
    project: &str,
    lock_secs: i64,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let tx = conn.transaction()?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT lock_epoch FROM summarize_locks WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(epoch) = existing {
        if now - epoch < lock_secs.max(1) {
            tx.rollback()?;
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT INTO summarize_locks (project, lock_epoch)
         VALUES (?1, ?2)
         ON CONFLICT(project) DO UPDATE SET lock_epoch = ?2",
        params![project, now],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn release_summarize_lock(conn: &Connection, project: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM summarize_locks WHERE project = ?1",
        params![project],
    )?;
    Ok(())
}
