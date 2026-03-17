use anyhow::Result;
use rusqlite::{params, Connection};

/// Record AI usage event to database for cost tracking.
pub fn record_ai_usage(
    conn: &Connection,
    project: Option<&str>,
    operation: &str,
    executor: &str,
    model: Option<&str>,
    input_tokens: i64,
    output_tokens: i64,
    estimated_cost_usd: f64,
) -> Result<()> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();
    let total_tokens = input_tokens + output_tokens;

    conn.execute(
        "INSERT INTO ai_usage_events \
         (created_at, created_at_epoch, project, operation, executor, model, \
          input_tokens, output_tokens, total_tokens, estimated_cost_usd) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            created_at,
            created_at_epoch,
            project,
            operation,
            executor,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            estimated_cost_usd
        ],
    )?;
    Ok(())
}
