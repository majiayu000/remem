use anyhow::Result;
use rusqlite::{params, Connection};

use crate::ai::TokenUsage;

/// Record AI usage event to database for cost tracking.
pub(crate) fn record_ai_usage(
    conn: &Connection,
    project: Option<&str>,
    session_id: Option<&str>,
    operation: &str,
    executor: &str,
    model: Option<&str>,
    usage: &TokenUsage,
    usage_source: &str,
    pricing_source: &str,
    estimated_cost_usd: f64,
) -> Result<()> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    conn.execute(
        "INSERT INTO ai_usage_events \
         (created_at, created_at_epoch, project, session_id, operation, executor, model, \
          input_tokens, output_tokens, reasoning_tokens, cache_creation_tokens, \
          cache_read_tokens, raw_input_tokens, raw_output_tokens, total_tokens, \
          estimated_cost_usd, usage_source, pricing_source) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            created_at,
            created_at_epoch,
            project,
            session_id,
            operation,
            executor,
            model,
            usage.input_tokens,
            usage.output_tokens,
            usage.reasoning_tokens,
            usage.cache_creation_tokens,
            usage.cache_read_tokens,
            usage.raw_input_tokens,
            usage.raw_output_tokens,
            usage.total_tokens(),
            estimated_cost_usd,
            usage_source,
            pricing_source
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn record_ai_usage_persists_session_id() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let usage = TokenUsage::estimated(100, 25);

        record_ai_usage(
            &conn,
            Some("/repo"),
            Some("sess-status-spend"),
            "summarize",
            "codex-cli",
            Some("codex-default"),
            &usage,
            "text_estimate",
            "remem_static",
            0.001,
        )?;

        let stored: (Option<String>, i64) = conn.query_row(
            "SELECT session_id, total_tokens FROM ai_usage_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(stored, (Some("sess-status-spend".to_string()), 125));
        Ok(())
    }
}
