use anyhow::Result;

use crate::db;

pub(in crate::cli) fn run_status() -> Result<()> {
    let conn = db::open_db()?;
    let db_path = db::db_path();
    let db_size = std::fs::metadata(&db_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let version = env!("CARGO_PKG_VERSION");
    let stats = db::query_system_stats(&conn)?;

    let today_start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp())
        .unwrap_or(0);
    let daily_stats = db::query_daily_activity_stats(&conn, today_start)?;
    let top_projects = db::query_top_projects(&conn, 5)?;

    println!("remem v{}", version);
    println!(
        "Database: {} ({:.1} MB)",
        db_path.display(),
        db_size as f64 / 1_048_576.0
    );
    println!();
    println!("  Memories:      {:>6}", stats.active_memories);
    println!("  Observations:  {:>6}", stats.active_observations);
    println!("  Sessions:      {:>6}", stats.session_summaries);
    println!("  Pending:       {:>6}", stats.pending_observations);
    println!("  Pending failed:{:>6}", stats.failed_pending_observations);
    println!();
    println!("Today:");
    println!("  New memories:      {:>4}", daily_stats.memories);
    println!("  New observations:  {:>4}", daily_stats.observations);

    if !top_projects.is_empty() {
        println!();
        println!("Top projects:");
        for project in &top_projects {
            println!("  {:>4}  {}", project.count, project.project);
        }
    }

    Ok(())
}
