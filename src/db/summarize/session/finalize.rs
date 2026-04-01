use anyhow::Result;
use rusqlite::{params, Connection};

pub fn finalize_summarize(
    conn: &mut Connection,
    memory_session_id: &str,
    project: &str,
    message_hash: &str,
    request: Option<&str>,
    completed: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    next_steps: Option<&str>,
    preferences: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<usize> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    let tx = conn.transaction()?;
    let deleted = tx.execute(
        "DELETE FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
        params![memory_session_id, project],
    )?;
    tx.execute(
        "INSERT INTO session_summaries \
         (memory_session_id, project, request, completed, decisions, learned, \
          next_steps, preferences, prompt_number, created_at, created_at_epoch, discovery_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            memory_session_id,
            project,
            request,
            completed,
            decisions,
            learned,
            next_steps,
            preferences,
            prompt_number,
            created_at,
            created_at_epoch,
            discovery_tokens
        ],
    )?;
    tx.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?2,
           last_message_hash = ?3",
        params![project, created_at_epoch, message_hash],
    )?;
    tx.commit()?;
    Ok(deleted)
}
