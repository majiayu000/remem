use std::fmt::Write;

use anyhow::Result;
use rusqlite::Connection;

use super::detail::{query_monthly, query_recent_observations};
use super::summary::{query_overview, query_token_economics, query_type_counts};

fn render_recent_timeline(out: &mut String, conn: &Connection, project: &str) -> Result<()> {
    writeln!(out, "## Timeline (recent first)")?;
    let recent = query_recent_observations(conn, project, 200)?;

    let mut current_date = String::new();
    for observation in recent {
        let date = chrono::DateTime::from_timestamp(observation.created_at_epoch, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        if date != current_date {
            writeln!(out, "### {}", date)?;
            current_date = date;
        }
        let title = observation.title.as_deref().unwrap_or("(untitled)");
        writeln!(
            out,
            "- #{} [{}] {}",
            observation.id, observation.obs_type, title
        )?;
    }
    writeln!(out)?;
    Ok(())
}

fn render_monthly_breakdown(out: &mut String, conn: &Connection, project: &str) -> Result<()> {
    let monthly = query_monthly(conn, project)?;
    writeln!(out, "## Monthly Breakdown")?;
    writeln!(out, "| Month | Observations | Sessions | AI Cost |")?;
    writeln!(out, "|-------|-------------|----------|---------|")?;
    for month in &monthly {
        writeln!(
            out,
            "| {} | {} | {} | ${:.2} |",
            month.month, month.observations, month.sessions, month.ai_cost
        )?;
    }
    Ok(())
}

pub fn generate_timeline_report(conn: &Connection, project: &str, full: bool) -> Result<String> {
    let overview = query_overview(conn, project)?;
    let type_counts = query_type_counts(conn, project)?;
    let token_econ = query_token_economics(conn, project)?;

    let mut out = String::with_capacity(4096);
    writeln!(out, "# Journey Into {}\n", project)?;

    writeln!(out, "## Overview")?;
    writeln!(
        out,
        "- Time span: {} -> {} ({} days)",
        overview.first_date, overview.last_date, overview.days_span
    )?;
    writeln!(out, "- Total observations: {}", overview.total_observations)?;
    writeln!(out, "- Total sessions: {}", overview.total_sessions)?;
    writeln!(out, "- Total memories: {}\n", overview.total_memories)?;

    writeln!(out, "## Activity by Type")?;
    let total = overview.total_observations.max(1) as f64;
    for type_count in &type_counts {
        let pct = (type_count.count as f64 / total * 100.0).round() as i64;
        writeln!(
            out,
            "- {}: {} ({}%)",
            type_count.obs_type, type_count.count, pct
        )?;
    }
    writeln!(out)?;

    writeln!(out, "## Token Economics")?;
    writeln!(out, "- Total AI cost: ${:.2}", token_econ.total_ai_cost)?;
    let discovery_m = token_econ.total_discovery_tokens as f64 / 1_000_000.0;
    writeln!(out, "- Total discovery tokens: {:.1}M", discovery_m)?;
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

    if full {
        render_recent_timeline(&mut out, conn, project)?;
        render_monthly_breakdown(&mut out, conn, project)?;
    }

    Ok(out)
}
