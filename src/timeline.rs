use anyhow::Result;
use rusqlite::Connection;
use std::fmt::Write;

use crate::db::to_sql_refs;
use crate::db_query::push_project_filter;

struct Overview {
    first_date: String,
    last_date: String,
    days_span: i64,
    total_observations: i64,
    total_sessions: i64,
    total_memories: i64,
}

struct TypeCount {
    obs_type: String,
    count: i64,
}

struct MonthRow {
    month: String,
    observations: i64,
    sessions: i64,
    ai_cost: f64,
}

struct TokenEcon {
    total_ai_cost: f64,
    total_discovery_tokens: i64,
    sessions_with_context: i64,
}

fn query_overview(conn: &Connection, project: &str) -> Result<Overview> {
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
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
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
        |r| r.get(0),
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
        |r| r.get(0),
    )?;

    let fmt = |epoch: i64| -> String {
        chrono::DateTime::from_timestamp(epoch, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into())
    };

    let days = if max_epoch > min_epoch {
        (max_epoch - min_epoch) / 86400 + 1
    } else {
        0
    };

    Ok(Overview {
        first_date: fmt(min_epoch),
        last_date: fmt(max_epoch),
        days_span: days,
        total_observations: total_obs,
        total_sessions,
        total_memories,
    })
}

fn query_type_counts(conn: &Connection, project: &str) -> Result<Vec<TypeCount>> {
    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, _) = push_project_filter("project", project, 1, &mut p);
    let refs = to_sql_refs(&p);

    let mut stmt = conn.prepare(&format!(
        "SELECT type, COUNT(*) as cnt FROM observations WHERE {} GROUP BY type ORDER BY cnt DESC",
        pf
    ))?;
    let rows = stmt.query_map(refs.as_slice(), |r| {
        Ok(TypeCount {
            obs_type: r.get(0)?,
            count: r.get(1)?,
        })
    })?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

fn query_token_economics(conn: &Connection, project: &str) -> Result<TokenEcon> {
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
            |r| r.get(0),
        )
        .unwrap_or(0.0);

    let mut p2: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf2, _) = push_project_filter("project", project, 1, &mut p2);
    let refs2 = to_sql_refs(&p2);
    let total_discovery: i64 = conn
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(discovery_tokens), 0) FROM observations WHERE {}",
                pf2
            ),
            refs2.as_slice(),
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut p3: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf3, _) = push_project_filter("project", project, 1, &mut p3);
    let refs3 = to_sql_refs(&p3);
    let sessions_ctx: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT memory_session_id) FROM session_summaries WHERE {}",
                pf3
            ),
            refs3.as_slice(),
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(TokenEcon {
        total_ai_cost,
        total_discovery_tokens: total_discovery,
        sessions_with_context: sessions_ctx,
    })
}

fn query_monthly(conn: &Connection, project: &str) -> Result<Vec<MonthRow>> {
    let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf, _) = push_project_filter("o.project", project, 1, &mut p);
    let refs = to_sql_refs(&p);

    let sql = format!(
        "SELECT strftime('%Y-%m', o.created_at_epoch, 'unixepoch') AS month, \
         COUNT(*) AS obs \
         FROM observations o WHERE {} GROUP BY month ORDER BY month DESC",
        pf
    );
    let mut stmt = conn.prepare(&sql)?;
    let obs_rows = stmt.query_map(refs.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut months: std::collections::BTreeMap<String, MonthRow> =
        std::collections::BTreeMap::new();
    for row in obs_rows {
        let (m, cnt) = row?;
        months
            .entry(m.clone())
            .or_insert(MonthRow {
                month: m,
                observations: 0,
                sessions: 0,
                ai_cost: 0.0,
            })
            .observations = cnt;
    }

    let mut p2: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf2, _) = push_project_filter("project", project, 1, &mut p2);
    let refs2 = to_sql_refs(&p2);
    let sql2 = format!(
        "SELECT strftime('%Y-%m', created_at_epoch, 'unixepoch') AS month, \
         COUNT(DISTINCT memory_session_id) \
         FROM session_summaries WHERE {} GROUP BY month",
        pf2
    );
    let mut stmt2 = conn.prepare(&sql2)?;
    let sess_rows = stmt2.query_map(refs2.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in sess_rows {
        let (m, cnt) = row?;
        months
            .entry(m.clone())
            .or_insert(MonthRow {
                month: m,
                observations: 0,
                sessions: 0,
                ai_cost: 0.0,
            })
            .sessions = cnt;
    }

    let mut p3: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (pf3, _) = push_project_filter("project", project, 1, &mut p3);
    let refs3 = to_sql_refs(&p3);
    let sql3 = format!(
        "SELECT strftime('%Y-%m', created_at_epoch, 'unixepoch') AS month, \
         COALESCE(SUM(estimated_cost_usd), 0.0) \
         FROM ai_usage_events WHERE {} GROUP BY month",
        pf3
    );
    let mut stmt3 = conn.prepare(&sql3)?;
    let cost_rows = stmt3.query_map(refs3.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    })?;
    for row in cost_rows {
        let (m, cost) = row?;
        months
            .entry(m.clone())
            .or_insert(MonthRow {
                month: m,
                observations: 0,
                sessions: 0,
                ai_cost: 0.0,
            })
            .ai_cost = cost;
    }

    let mut result: Vec<MonthRow> = months.into_values().collect();
    result.sort_by(|a, b| b.month.cmp(&a.month));
    Ok(result)
}

/// Generate a project timeline report in Markdown format.
pub fn generate_timeline_report(conn: &Connection, project: &str, full: bool) -> Result<String> {
    let overview = query_overview(conn, project)?;
    let type_counts = query_type_counts(conn, project)?;
    let token_econ = query_token_economics(conn, project)?;

    let mut out = String::with_capacity(4096);

    // Header
    writeln!(out, "# Journey Into {}\n", project)?;

    // Overview
    writeln!(out, "## Overview")?;
    writeln!(
        out,
        "- Time span: {} -> {} ({} days)",
        overview.first_date, overview.last_date, overview.days_span
    )?;
    writeln!(out, "- Total observations: {}", overview.total_observations)?;
    writeln!(out, "- Total sessions: {}", overview.total_sessions)?;
    writeln!(out, "- Total memories: {}\n", overview.total_memories)?;

    // Activity by Type
    writeln!(out, "## Activity by Type")?;
    let total = overview.total_observations.max(1) as f64;
    for tc in &type_counts {
        let pct = (tc.count as f64 / total * 100.0).round() as i64;
        writeln!(out, "- {}: {} ({}%)", tc.obs_type, tc.count, pct)?;
    }
    writeln!(out)?;

    // Token Economics
    writeln!(out, "## Token Economics")?;
    writeln!(out, "- Total AI cost: ${:.2}", token_econ.total_ai_cost)?;
    let disc_m = token_econ.total_discovery_tokens as f64 / 1_000_000.0;
    writeln!(out, "- Total discovery tokens: {:.1}M", disc_m)?;
    writeln!(
        out,
        "- Sessions with context injection: {}",
        token_econ.sessions_with_context
    )?;
    let recall_savings = token_econ.sessions_with_context * 300;
    writeln!(
        out,
        "- Estimated passive recall savings: ~{}K tokens\n",
        recall_savings
    )?;

    // Full mode: Timeline + Monthly breakdown
    if full {
        // Timeline (recent first, grouped by date)
        writeln!(out, "## Timeline (recent first)")?;
        let mut pv: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let (pf, idx) = push_project_filter("project", project, 1, &mut pv);
        pv.push(Box::new(200_i64));
        let sql = format!(
            "SELECT id, type, title, created_at_epoch FROM observations \
             WHERE {} ORDER BY created_at_epoch DESC LIMIT ?{}",
            pf, idx
        );
        let refs = to_sql_refs(&pv);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;

        let mut current_date = String::new();
        for row in rows {
            let (id, obs_type, title, epoch) = row?;
            let date = chrono::DateTime::from_timestamp(epoch, 0)
                .map(|dt| dt.format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            if date != current_date {
                writeln!(out, "### {}", date)?;
                current_date = date;
            }
            let title_str = title.as_deref().unwrap_or("(untitled)");
            writeln!(out, "- #{} [{}] {}", id, obs_type, title_str)?;
        }
        writeln!(out)?;

        // Monthly Breakdown
        let monthly = query_monthly(conn, project)?;
        writeln!(out, "## Monthly Breakdown")?;
        writeln!(out, "| Month | Observations | Sessions | AI Cost |")?;
        writeln!(out, "|-------|-------------|----------|---------|")?;
        for m in &monthly {
            writeln!(
                out,
                "| {} | {} | {} | ${:.2} |",
                m.month, m.observations, m.sessions, m.ai_cost
            )?;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn setup_test_db(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE observations (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                type TEXT NOT NULL,
                title TEXT,
                subtitle TEXT,
                narrative TEXT,
                facts TEXT,
                concepts TEXT,
                files_read TEXT,
                files_modified TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0,
                status TEXT DEFAULT 'active',
                last_accessed_epoch INTEGER
            );
            CREATE TABLE session_summaries (
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
            CREATE TABLE memories (
                id INTEGER PRIMARY KEY,
                session_id TEXT,
                project TEXT NOT NULL,
                topic_key TEXT,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                files TEXT,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'active'
            );
            CREATE TABLE ai_usage_events (
                id INTEGER PRIMARY KEY,
                created_at TEXT NOT NULL,
                created_at_epoch INTEGER NOT NULL,
                project TEXT,
                operation TEXT NOT NULL,
                executor TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                estimated_cost_usd REAL NOT NULL
            );",
        )
        .unwrap();
    }

    #[test]
    fn empty_project_produces_report() {
        let conn = Connection::open_in_memory().unwrap();
        setup_test_db(&conn);

        let report = generate_timeline_report(&conn, "tools/remem", false).unwrap();
        assert!(report.contains("# Journey Into tools/remem"));
        assert!(report.contains("Total observations: 0"));
    }

    #[test]
    fn summary_report_excludes_timeline() {
        let conn = Connection::open_in_memory().unwrap();
        setup_test_db(&conn);
        let now = chrono::Utc::now().timestamp();

        conn.execute(
            "INSERT INTO observations (memory_session_id, project, type, title, created_at_epoch, discovery_tokens) \
             VALUES ('s1', 'tools/remem', 'decision', 'Test observation', ?1, 100)",
            params![now],
        ).unwrap();

        let report = generate_timeline_report(&conn, "tools/remem", false).unwrap();
        assert!(report.contains("Total observations: 1"));
        assert!(!report.contains("## Timeline"));
        assert!(!report.contains("## Monthly Breakdown"));
    }

    #[test]
    fn full_report_includes_timeline_and_monthly() {
        let conn = Connection::open_in_memory().unwrap();
        setup_test_db(&conn);
        let now = chrono::Utc::now().timestamp();

        conn.execute(
            "INSERT INTO observations (memory_session_id, project, type, title, created_at_epoch, discovery_tokens) \
             VALUES ('s1', 'tools/remem', 'decision', 'FTS5 switch', ?1, 500)",
            params![now],
        ).unwrap();
        conn.execute(
            "INSERT INTO observations (memory_session_id, project, type, title, created_at_epoch, discovery_tokens) \
             VALUES ('s1', 'tools/remem', 'bugfix', 'Fix search', ?1, 300)",
            params![now],
        ).unwrap();
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch) \
             VALUES ('s1', 'tools/remem', 'analyze search', '2026-03-19', ?1)",
            params![now],
        ).unwrap();
        conn.execute(
            "INSERT INTO memories (session_id, project, title, content, memory_type, created_at_epoch, updated_at_epoch) \
             VALUES ('s1', 'tools/remem', 'Test memory', 'content', 'decision', ?1, ?1)",
            params![now],
        ).unwrap();

        let report = generate_timeline_report(&conn, "tools/remem", true).unwrap();
        assert!(report.contains("## Timeline (recent first)"));
        assert!(report.contains("[decision] FTS5 switch"));
        assert!(report.contains("[bugfix] Fix search"));
        assert!(report.contains("## Monthly Breakdown"));
        assert!(report.contains("Total observations: 2"));
        assert!(report.contains("Total sessions: 1"));
        assert!(report.contains("Total memories: 1"));
    }
}
