use anyhow::Result;
use rusqlite::{params, Connection};

pub fn upsert_session(
    conn: &Connection,
    content_session_id: &str,
    project: &str,
    user_prompt: Option<&str>,
) -> Result<String> {
    let now = chrono::Utc::now();
    let started_at = now.to_rfc3339();
    let started_at_epoch = now.timestamp();
    let memory_session_id = format!(
        "mem-{}",
        super::super::super::core::truncate_str(content_session_id, 8)
    );

    conn.execute(
        "INSERT INTO sdk_sessions \
         (content_session_id, memory_session_id, project, user_prompt, \
          started_at, started_at_epoch, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active') \
         ON CONFLICT(content_session_id) DO UPDATE SET \
         prompt_counter = prompt_counter + 1",
        params![
            content_session_id,
            memory_session_id,
            project,
            user_prompt,
            started_at,
            started_at_epoch
        ],
    )?;

    let mid: String = conn.query_row(
        "SELECT memory_session_id FROM sdk_sessions WHERE content_session_id = ?1",
        params![content_session_id],
        |row| row.get(0),
    )?;
    Ok(mid)
}
