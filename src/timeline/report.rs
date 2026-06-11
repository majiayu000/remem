use std::fmt::Write;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use super::detail::{query_monthly, query_recent_observations};
use super::summary::{query_overview, query_token_economics, query_type_counts};
use super::types::{MonthRow, Overview, RecentObservation, TokenEcon, TypeCount};

#[derive(Debug, Serialize)]
pub(crate) struct TimelineReportData {
    project: String,
    full: bool,
    overview: Overview,
    activity_by_type: Vec<TypeCount>,
    token_economics: TokenEcon,
    #[serde(skip_serializing_if = "Option::is_none")]
    recent_timeline: Option<Vec<RecentObservation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    monthly_breakdown: Option<Vec<MonthRow>>,
}

fn render_recent_timeline(out: &mut String, recent: &[RecentObservation]) -> Result<()> {
    writeln!(out, "## Timeline (recent first)")?;

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

fn render_monthly_breakdown(out: &mut String, monthly: &[MonthRow]) -> Result<()> {
    writeln!(out, "## Monthly Breakdown")?;
    writeln!(out, "| Month | Observations | Sessions | AI Cost |")?;
    writeln!(out, "|-------|-------------|----------|---------|")?;
    for month in monthly {
        writeln!(
            out,
            "| {} | {} | {} | ${:.2} |",
            month.month, month.observations, month.sessions, month.ai_cost
        )?;
    }
    Ok(())
}

pub(crate) fn generate_timeline_report_data(
    conn: &Connection,
    project: &str,
    full: bool,
) -> Result<TimelineReportData> {
    let overview = query_overview(conn, project)?;
    let activity_by_type = query_type_counts(conn, project)?;
    let token_economics = query_token_economics(conn, project)?;
    let recent_timeline = if full {
        Some(query_recent_observations(conn, project, 200)?)
    } else {
        None
    };
    let monthly_breakdown = if full {
        Some(query_monthly(conn, project)?)
    } else {
        None
    };

    Ok(TimelineReportData {
        project: project.to_string(),
        full,
        overview,
        activity_by_type,
        token_economics,
        recent_timeline,
        monthly_breakdown,
    })
}

pub fn generate_timeline_report(conn: &Connection, project: &str, full: bool) -> Result<String> {
    let report = generate_timeline_report_data(conn, project, full)?;
    let mut out = String::with_capacity(4096);
    writeln!(out, "# Journey Into {}\n", project)?;

    writeln!(out, "## Overview")?;
    writeln!(
        out,
        "- Time span: {} -> {} ({} days)",
        report.overview.first_date, report.overview.last_date, report.overview.days_span
    )?;
    writeln!(
        out,
        "- Total observations: {}",
        report.overview.total_observations
    )?;
    writeln!(out, "- Total sessions: {}", report.overview.total_sessions)?;
    writeln!(
        out,
        "- Total memories: {}\n",
        report.overview.total_memories
    )?;

    writeln!(out, "## Activity by Type")?;
    let total = report.overview.total_observations.max(1) as f64;
    for type_count in &report.activity_by_type {
        let pct = (type_count.count as f64 / total * 100.0).round() as i64;
        writeln!(
            out,
            "- {}: {} ({}%)",
            type_count.obs_type, type_count.count, pct
        )?;
    }
    writeln!(out)?;

    writeln!(out, "## Token Economics")?;
    writeln!(
        out,
        "- Total AI cost: ${:.2}",
        report.token_economics.total_ai_cost
    )?;
    let discovery_m = report.token_economics.total_discovery_tokens as f64 / 1_000_000.0;
    writeln!(out, "- Total discovery tokens: {:.1}M", discovery_m)?;
    writeln!(
        out,
        "- Sessions with context injection: {}",
        report.token_economics.sessions_with_context
    )?;
    let recall_savings = report.token_economics.sessions_with_context * 300;
    writeln!(
        out,
        "- Estimated passive recall savings: ~{}K tokens\n",
        recall_savings
    )?;

    if let Some(recent) = report.recent_timeline.as_deref() {
        render_recent_timeline(&mut out, recent)?;
    }
    if let Some(monthly) = report.monthly_breakdown.as_deref() {
        render_monthly_breakdown(&mut out, monthly)?;
    }

    Ok(out)
}
