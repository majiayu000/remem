use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq)]
pub struct LatestSessionMemorySpend {
    pub session_id: String,
    pub project: String,
    pub latest_context_epoch: i64,
    pub context_rows: i64,
    pub context_output_chars: i64,
    pub context_estimated_tokens: i64,
    pub context_emit_count: i64,
    pub context_suppress_count: i64,
    pub ai_usage_attribution: String,
    pub ai_calls: i64,
    pub ai_total_tokens: i64,
    pub ai_estimated_cost_usd: f64,
    pub ai_unattributed_legacy_calls: i64,
}

pub fn query_latest_session_memory_spend(
    conn: &Connection,
) -> Result<Option<LatestSessionMemorySpend>> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, "context_injections")? {
        return Ok(None);
    }

    let Some(session_id) = conn
        .query_row(
            "SELECT session_id
             FROM context_injections
             WHERE session_id IS NOT NULL
               AND trim(session_id) <> ''
             ORDER BY updated_at_epoch DESC, last_emitted_epoch DESC, id DESC
             LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };

    let (
        project,
        latest_context_epoch,
        context_rows,
        context_output_chars,
        context_emit_count,
        context_suppress_count,
    ) = conn.query_row(
        "SELECT
            (SELECT project
             FROM context_injections latest
                 WHERE latest.session_id = ?1
                 ORDER BY latest.updated_at_epoch DESC,
                          latest.last_emitted_epoch DESC,
                          latest.id DESC
                 LIMIT 1),
            COALESCE(MAX(updated_at_epoch), 0),
            COUNT(*),
            COALESCE(SUM(output_chars), 0),
            COALESCE(SUM(emit_count), 0),
            COALESCE(SUM(suppress_count), 0)
         FROM context_injections
         WHERE session_id = ?1",
        params![session_id.as_str()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;

    let (
        ai_usage_attribution,
        ai_calls,
        ai_total_tokens,
        ai_estimated_cost_usd,
        ai_unattributed_legacy_calls,
    ) = if sqlite_column_exists(conn, "ai_usage_events", "session_id")? {
        let (calls, total_tokens, estimated_cost_usd) = conn.query_row(
            "SELECT COUNT(*),
                        COALESCE(SUM(total_tokens), 0),
                        COALESCE(SUM(estimated_cost_usd), 0.0)
                 FROM ai_usage_events
                 WHERE session_id = ?1",
            params![session_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            },
        )?;
        let unattributed_legacy_calls = conn.query_row(
            "SELECT COUNT(*)
                 FROM ai_usage_events
                 WHERE session_id IS NULL",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let attribution = if unattributed_legacy_calls > 0 {
            "partial"
        } else {
            "attributed"
        };
        (
            attribution.to_string(),
            calls,
            total_tokens,
            estimated_cost_usd,
            unattributed_legacy_calls,
        )
    } else {
        ("unavailable".to_string(), 0, 0, 0.0, 0)
    };

    Ok(Some(LatestSessionMemorySpend {
        session_id,
        project,
        latest_context_epoch,
        context_rows,
        context_output_chars,
        context_estimated_tokens: estimate_tokens_from_chars(context_output_chars),
        context_emit_count,
        context_suppress_count,
        ai_usage_attribution,
        ai_calls,
        ai_total_tokens,
        ai_estimated_cost_usd,
        ai_unattributed_legacy_calls,
    }))
}

fn estimate_tokens_from_chars(chars: i64) -> i64 {
    if chars <= 0 {
        0
    } else {
        (chars + 3) / 4
    }
}

fn sqlite_column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, table)? {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info(?1)
            WHERE name = ?2
         )",
        params![table, column],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn setup_status_spend_schema(conn: &Connection, include_ai_session_id: bool) {
        let ai_session_id_column = if include_ai_session_id {
            "session_id TEXT,"
        } else {
            ""
        };
        conn.execute_batch(&format!(
            "CREATE TABLE context_injections (
                id INTEGER PRIMARY KEY,
                host TEXT NOT NULL,
                project TEXT NOT NULL,
                injection_key TEXT NOT NULL,
                session_id TEXT,
                context_hash TEXT NOT NULL,
                output_mode TEXT NOT NULL,
                output_chars INTEGER NOT NULL,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                last_emitted_epoch INTEGER NOT NULL,
                emit_count INTEGER NOT NULL DEFAULT 1,
                suppress_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE ai_usage_events (
                id INTEGER PRIMARY KEY,
                created_at TEXT NOT NULL,
                created_at_epoch INTEGER NOT NULL,
                project TEXT,
                {ai_session_id_column}
                operation TEXT NOT NULL,
                executor TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                estimated_cost_usd REAL NOT NULL
            );"
        ))
        .expect("status spend schema should be created");
    }

    #[test]
    fn latest_session_memory_spend_combines_context_and_ai_usage() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_status_spend_schema(&conn, true);
        conn.execute_batch(
            "INSERT INTO context_injections
             (host, project, injection_key, session_id, context_hash, output_mode, output_chars,
              created_at_epoch, updated_at_epoch, last_emitted_epoch, emit_count, suppress_count)
             VALUES
             ('codex-cli', '/old', 'old-key', 'sess-old', 'h1', 'full', 1200, 10, 10, 10, 1, 0),
             ('codex-cli', '/repo', 'key-a', 'sess-new', 'h2', 'full', 801, 20, 31, 30, 2, 1),
             ('codex-cli', '/repo', 'key-b', 'sess-new', 'h3', 'suppressed', 399, 21, 32, 29, 1, 3);
             INSERT INTO ai_usage_events
             (created_at, created_at_epoch, project, session_id, operation, executor, model,
              input_tokens, output_tokens, total_tokens, estimated_cost_usd)
             VALUES
             ('2026-06-18T00:00:00Z', 30, '/repo', 'sess-new', 'summarize', 'codex-cli',
              'codex-default', 100, 50, 150, 0.0015),
             ('2026-06-18T00:00:01Z', 31, '/repo', 'sess-new', 'memory_candidate', 'codex-cli',
              'codex-default', 60, 40, 100, 0.0010),
             ('2026-06-18T00:00:02Z', 32, '/repo', NULL, 'legacy', 'codex-cli',
              'codex-default', 999, 1, 1000, 9.0);",
        )?;

        let spend = query_latest_session_memory_spend(&conn)?
            .ok_or_else(|| anyhow::anyhow!("latest session spend"))?;

        assert_eq!(
            spend,
            LatestSessionMemorySpend {
                session_id: "sess-new".to_string(),
                project: "/repo".to_string(),
                latest_context_epoch: 32,
                context_rows: 2,
                context_output_chars: 1200,
                context_estimated_tokens: 300,
                context_emit_count: 3,
                context_suppress_count: 4,
                ai_usage_attribution: "partial".to_string(),
                ai_calls: 2,
                ai_total_tokens: 250,
                ai_estimated_cost_usd: 0.0025,
                ai_unattributed_legacy_calls: 1,
            }
        );
        Ok(())
    }

    #[test]
    fn latest_session_memory_spend_uses_updated_activity_for_suppressed_sessions() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_status_spend_schema(&conn, true);
        conn.execute_batch(
            "INSERT INTO context_injections
             (host, project, injection_key, session_id, context_hash, output_mode, output_chars,
              created_at_epoch, updated_at_epoch, last_emitted_epoch, emit_count, suppress_count)
             VALUES
             ('codex-cli', '/old', 'old-key', 'sess-old', 'h1', 'full', 1200, 10, 100, 100, 1, 0),
             ('codex-cli', '/repo', 'key-a', 'sess-new', 'h2', 'suppressed', 401, 20, 110, 90, 1, 2);",
        )?;

        let spend = query_latest_session_memory_spend(&conn)?
            .ok_or_else(|| anyhow::anyhow!("latest session spend"))?;

        assert_eq!(spend.session_id, "sess-new");
        assert_eq!(spend.latest_context_epoch, 110);
        assert_eq!(spend.context_suppress_count, 2);
        Ok(())
    }

    #[test]
    fn latest_session_memory_spend_tolerates_legacy_ai_usage_without_session_column() -> Result<()>
    {
        let conn = Connection::open_in_memory()?;
        setup_status_spend_schema(&conn, false);
        conn.execute_batch(
            "INSERT INTO context_injections
             (host, project, injection_key, session_id, context_hash, output_mode, output_chars,
              created_at_epoch, updated_at_epoch, last_emitted_epoch, emit_count, suppress_count)
             VALUES
             ('codex-cli', '/repo', 'key-a', 'sess-new', 'h2', 'full', 401, 20, 31, 30, 1, 0);",
        )?;

        let spend = query_latest_session_memory_spend(&conn)?
            .ok_or_else(|| anyhow::anyhow!("latest session spend"))?;

        assert_eq!(spend.ai_usage_attribution, "unavailable");
        assert_eq!(spend.ai_calls, 0);
        assert_eq!(spend.context_estimated_tokens, 101);
        Ok(())
    }

    #[test]
    fn latest_session_memory_spend_is_blank_without_context_rows() -> Result<()> {
        let conn = Connection::open_in_memory()?;

        assert!(query_latest_session_memory_spend(&conn)?.is_none());
        Ok(())
    }
}
