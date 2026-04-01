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

pub fn upsert_session(
    conn: &Connection,
    content_session_id: &str,
    project: &str,
    user_prompt: Option<&str>,
) -> Result<String> {
    let now = chrono::Utc::now();
    let started_at = now.to_rfc3339();
    let started_at_epoch = now.timestamp();
    let memory_session_id = format!("mem-{}", super::super::core::truncate_str(content_session_id, 8));

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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_summary_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE session_summaries (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                request TEXT,
                completed TEXT,
                decisions TEXT,
                learned TEXT,
                next_steps TEXT,
                preferences TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0
            );
            CREATE TABLE summarize_cooldown (
                project TEXT PRIMARY KEY,
                last_summarize_epoch INTEGER NOT NULL,
                last_message_hash TEXT
            );",
        )?;
        Ok(())
    }

    #[test]
    fn finalize_summarize_replaces_in_single_commit() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_summary_schema(&conn)?;
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch, discovery_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["mem-1", "proj", "old", "2026-01-01T00:00:00Z", 1_i64, 10_i64],
        )?;

        let deleted = finalize_summarize(
            &mut conn,
            "mem-1",
            "proj",
            "hash-1",
            Some("new"),
            Some("done"),
            Some("decision"),
            Some("learned"),
            Some("next"),
            Some("pref"),
            None,
            99,
        )?;
        assert_eq!(deleted, 1);

        let request: String = conn.query_row(
            "SELECT request FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
            params!["mem-1", "proj"],
            |row| row.get(0),
        )?;
        assert_eq!(request, "new");

        let hash: String = conn.query_row(
            "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
            params!["proj"],
            |row| row.get(0),
        )?;
        assert_eq!(hash, "hash-1");
        Ok(())
    }
}
