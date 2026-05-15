use anyhow::{bail, Result};

use crate::db::{self, AiUsageSourceTotals, AiUsageTotals};

const SECS_PER_DAY: i64 = 86_400;
const SECS_PER_WEEK: i64 = 7 * SECS_PER_DAY;

pub(in crate::cli) fn run_usage(project: Option<&str>, days: i64, weeks: i64) -> Result<()> {
    validate_window("days", days)?;
    validate_window("weeks", weeks)?;

    let conn = db::open_db()?;
    let now = chrono::Utc::now().timestamp();
    let daily_since = now.saturating_sub(days.saturating_mul(SECS_PER_DAY));
    let weekly_since = now.saturating_sub(weeks.saturating_mul(SECS_PER_WEEK));
    let totals = db::query_ai_usage_totals(&conn, Some(weekly_since), project)?;
    let source_totals = db::query_ai_usage_source_totals(&conn, Some(weekly_since), project)?;
    let daily = db::query_daily_ai_usage(&conn, daily_since, project, days)?;
    let weekly = db::query_weekly_ai_usage(&conn, weekly_since, project, weeks)?;

    println!("Token usage");
    match project {
        Some(project) => println!("Project: {}", project),
        None => println!("Project: all"),
    }
    println!();
    print_totals(&format!("Last {weeks} weeks"), &totals);
    print_precision(&source_totals);
    println!();
    print_daily(days, &daily);
    println!();
    print_weekly(weeks, &weekly);

    Ok(())
}

fn print_precision(rows: &[AiUsageSourceTotals]) {
    if rows.is_empty() {
        return;
    }

    let estimated_calls: i64 = rows
        .iter()
        .filter(|row| row.usage_source == "text_estimate")
        .map(|row| row.calls)
        .sum();
    let exact_calls: i64 = rows
        .iter()
        .filter(|row| row.usage_source != "text_estimate")
        .map(|row| row.calls)
        .sum();
    let estimated_tokens: i64 = rows
        .iter()
        .filter(|row| row.usage_source == "text_estimate")
        .map(|row| row.total_tokens)
        .sum();

    println!(
        "  Usage precision:       {:>12} exact/provider/log calls, {:>12} text-estimate calls",
        exact_calls, estimated_calls
    );
    if estimated_tokens > 0 {
        println!(
            "  Precision note:        {:>12} tokens are text estimates, not provider invoice data",
            estimated_tokens
        );
    }
}

fn validate_window(name: &str, value: i64) -> Result<()> {
    if value < 1 {
        bail!("{name} must be at least 1");
    }
    Ok(())
}

fn print_totals(label: &str, totals: &AiUsageTotals) {
    println!("{label}:");
    println!("  Calls:                 {:>12}", totals.calls);
    println!("  Input tokens:          {:>12}", totals.input_tokens);
    println!(
        "  Cache creation tokens: {:>12}",
        totals.cache_creation_tokens
    );
    println!("  Cache read tokens:     {:>12}", totals.cache_read_tokens);
    println!("  Output tokens:         {:>12}", totals.output_tokens);
    println!("  Reasoning tokens:      {:>12}", totals.reasoning_tokens);
    println!("  Total tokens:          {:>12}", totals.total_tokens);
    println!(
        "  Est. cost:             ${:>11.4}",
        totals.estimated_cost_usd
    );
}

fn print_daily(days: i64, rows: &[db::DailyAiUsage]) {
    println!("Daily (last {days} days):");
    if rows.is_empty() {
        println!("  No usage events");
        return;
    }
    print_header("Day");
    for row in rows {
        print_row(
            &row.day,
            row.calls,
            row.input_tokens,
            row.cache_creation_tokens + row.cache_read_tokens,
            row.output_tokens,
            row.reasoning_tokens,
            row.total_tokens,
            row.estimated_cost_usd,
        );
    }
}

fn print_weekly(weeks: i64, rows: &[db::WeeklyAiUsage]) {
    println!("Weekly (last {weeks} weeks):");
    if rows.is_empty() {
        println!("  No usage events");
        return;
    }
    print_header("Week");
    for row in rows {
        print_row(
            &row.week,
            row.calls,
            row.input_tokens,
            row.cache_creation_tokens + row.cache_read_tokens,
            row.output_tokens,
            row.reasoning_tokens,
            row.total_tokens,
            row.estimated_cost_usd,
        );
    }
}

fn print_header(label: &str) {
    println!(
        "  {:<12} {:>7} {:>11} {:>11} {:>11} {:>11} {:>12} {:>10}",
        label, "Calls", "Input", "Cache", "Output", "Reasoning", "Total", "Cost"
    );
}

fn print_row(
    label: &str,
    calls: i64,
    input_tokens: i64,
    cache_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
    total_tokens: i64,
    estimated_cost_usd: f64,
) {
    println!(
        "  {:<12} {:>7} {:>11} {:>11} {:>11} {:>11} {:>12} ${:>9.4}",
        label,
        calls,
        input_tokens,
        cache_tokens,
        output_tokens,
        reasoning_tokens,
        total_tokens,
        estimated_cost_usd
    );
}
