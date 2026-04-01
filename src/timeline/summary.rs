use anyhow::Result;
use rusqlite::Connection;

use crate::db::to_sql_refs;
use crate::db_query::push_project_filter;

use super::types::{Overview, TokenEcon, TypeCount};

pub(super) fn query_overview(conn: &Connection, project: &str) -> Result<Overview> {
    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, _) = push_project_filter("project", project, 1, &mut p);
    let refs = to_sql_refs(&p);
    let (min_epoch, max_epoch, total_obs): (i64, i64, i64) = conn.query_row(
        &format!(
            "SELECT COALESCE(MIN(created_at_epoch),0), COALESCE(MAX(created_at_epoch),0), COUNT(*) \
             FROM observations WHERE {}",
            pf
        ),
        refs.as_slice(),
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    let mut p2: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf2, _) = push_project_filter("project", project, 1, &mut p2);
    let refs2 = to_sql_refs(&p2);
    let total_sessions: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT memory_session_id) FROM session_summaries WHERE {}",
            pf2
        ),
        refs2.as_slice(),
        |row| row.get(0),
    )?;

    let mut p3: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf3, _) = push_project_filter("project", project, 1, &mut p3);
    let refs3 = to_sql_refs(&p3);
    let total_memories: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM memories WHERE {} AND status = 'active'",
            pf3
        ),
        refs3.as_slice(),
        |row| row.get(0),
    )?;

    let format_epoch = |epoch: i64| -> String {
        chrono::DateTime::from_timestamp(epoch, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into())
    };
    let days_span = if max_epoch > min_epoch {
        (max_epoch - min_epoch) / 86400 + 1
    } else {
        0
    };

    Ok(Overview {
        first_date: format_epoch(min_epoch),
        last_date: format_epoch(max_epoch),
        days_span,
        total_observations: total_obs,
        total_sessions,
        total_memories,
    })
}

pub(super) fn query_type_counts(conn: &Connection, project: &str) -> Result<Vec<TypeCount>> {
    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, _) = push_project_filter("project", project, 1, &mut p);
    let refs = to_sql_refs(&p);

    let mut stmt = conn.prepare(&format!(
        "SELECT type, COUNT(*) as cnt FROM observations WHERE {} GROUP BY type ORDER BY cnt DESC",
        pf
    ))?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(TypeCount {
            obs_type: row.get(0)?,
            count: row.get(1)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub(super) fn query_token_economics(conn: &Connection, project: &str) -> Result<TokenEcon> {
    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, _) = push_project_filter("project", project, 1, &mut p);
    let refs = to_sql_refs(&p);
    let total_ai_cost: f64 = conn
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(estimated_cost_usd), 0.0) FROM ai_usage_events WHERE {}",
                pf
            ),
            refs.as_slice(),
            |row| row.get(0),
        )
        .unwrap_or(0.0);

    let mut p2: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf2, _) = push_project_filter("project", project, 1, &mut p2);
    let refs2 = to_sql_refs(&p2);
    let total_discovery_tokens: i64 = conn
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(discovery_tokens), 0) FROM observations WHERE {}",
                pf2
            ),
            refs2.as_slice(),
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut p3: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf3, _) = push_project_filter("project", project, 1, &mut p3);
    let refs3 = to_sql_refs(&p3);
    let sessions_with_context: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT memory_session_id) FROM session_summaries WHERE {}",
                pf3
            ),
            refs3.as_slice(),
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok(TokenEcon {
        total_ai_cost,
        total_discovery_tokens,
        sessions_with_context,
    })
}
