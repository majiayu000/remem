use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::Connection;

use crate::db::to_sql_refs;
use crate::db_query::push_project_filter;

use super::types::{MonthRow, RecentObservation};

pub(super) fn query_monthly(conn: &Connection, project: &str) -> Result<Vec<MonthRow>> {
    let mut months: BTreeMap<String, MonthRow> = BTreeMap::new();

    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, _) = push_project_filter("o.project", project, 1, &mut p);
    let refs = to_sql_refs(&p);
    let mut stmt = conn.prepare(&format!(
        "SELECT strftime('%Y-%m', o.created_at_epoch, 'unixepoch') AS month, COUNT(*) AS obs \
         FROM observations o WHERE {} GROUP BY month ORDER BY month DESC",
        pf
    ))?;
    let obs_rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in obs_rows {
        let (month, observations) = row?;
        months
            .entry(month.clone())
            .or_insert(MonthRow {
                month,
                observations: 0,
                sessions: 0,
                ai_cost: 0.0,
            })
            .observations = observations;
    }

    let mut p2: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf2, _) = push_project_filter("project", project, 1, &mut p2);
    let refs2 = to_sql_refs(&p2);
    let mut stmt2 = conn.prepare(&format!(
        "SELECT strftime('%Y-%m', created_at_epoch, 'unixepoch') AS month, \
         COUNT(DISTINCT memory_session_id) FROM session_summaries WHERE {} GROUP BY month",
        pf2
    ))?;
    let sess_rows = stmt2.query_map(refs2.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in sess_rows {
        let (month, sessions) = row?;
        months
            .entry(month.clone())
            .or_insert(MonthRow {
                month,
                observations: 0,
                sessions: 0,
                ai_cost: 0.0,
            })
            .sessions = sessions;
    }

    let mut p3: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf3, _) = push_project_filter("project", project, 1, &mut p3);
    let refs3 = to_sql_refs(&p3);
    let mut stmt3 = conn.prepare(&format!(
        "SELECT strftime('%Y-%m', created_at_epoch, 'unixepoch') AS month, \
         COALESCE(SUM(estimated_cost_usd), 0.0) FROM ai_usage_events WHERE {} GROUP BY month",
        pf3
    ))?;
    let cost_rows = stmt3.query_map(refs3.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    })?;
    for row in cost_rows {
        let (month, ai_cost) = row?;
        months
            .entry(month.clone())
            .or_insert(MonthRow {
                month,
                observations: 0,
                sessions: 0,
                ai_cost: 0.0,
            })
            .ai_cost = ai_cost;
    }

    let mut result: Vec<MonthRow> = months.into_values().collect();
    result.sort_by(|a, b| b.month.cmp(&a.month));
    Ok(result)
}

pub(super) fn query_recent_observations(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<RecentObservation>> {
    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, idx) = push_project_filter("project", project, 1, &mut p);
    p.push(Box::new(limit));
    let refs = to_sql_refs(&p);
    let mut stmt = conn.prepare(&format!(
        "SELECT id, type, title, created_at_epoch FROM observations \
         WHERE {} ORDER BY created_at_epoch DESC LIMIT ?{}",
        pf, idx
    ))?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(RecentObservation {
            id: row.get(0)?,
            obs_type: row.get(1)?,
            title: row.get(2)?,
            created_at_epoch: row.get(3)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}
