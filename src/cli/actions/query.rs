use anyhow::Result;
use rusqlite::Connection;

use crate::{db, memory};

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

pub(in crate::cli) fn run_search(
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
) -> Result<()> {
    let conn = db::open_db()?;
    let results = crate::search::search(&conn, Some(query), project, memory_type, limit, 0, false)?;

    if results.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    println!("Found {} result(s):\n", results.len());
    for memory in &results {
        let date = chrono::DateTime::from_timestamp(memory.created_at_epoch, 0)
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let preview = memory
            .text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();
        println!(
            "  [{}] {} | {} | {} | {}",
            memory.id, memory.memory_type, memory.project, date, memory.title
        );
        if !preview.is_empty() && preview != memory.title {
            println!("       {}", preview);
        }
    }

    Ok(())
}

pub(in crate::cli) fn run_show(id: i64) -> Result<()> {
    let conn = db::open_db()?;
    let memories = memory::get_memories_by_ids(&conn, &[id], None)?;

    let Some(memory) = memories.first() else {
        println!("Memory {} not found.", id);
        return Ok(());
    };

    let created = chrono::DateTime::from_timestamp(memory.created_at_epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();
    let updated = chrono::DateTime::from_timestamp(memory.updated_at_epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();

    println!("ID:       {}", memory.id);
    println!("Title:    {}", memory.title);
    println!("Type:     {}", memory.memory_type);
    println!("Project:  {}", memory.project);
    println!("Scope:    {}", memory.scope);
    println!("Status:   {}", memory.status);
    if let Some(topic_key) = &memory.topic_key {
        println!("Topic:    {}", topic_key);
    }
    if let Some(branch) = &memory.branch {
        println!("Branch:   {}", branch);
    }
    println!("Created:  {}", created);
    println!("Updated:  {}", updated);
    println!();
    println!("{}", memory.text);

    Ok(())
}

pub(in crate::cli) fn run_backfill_entities() -> Result<()> {
    let conn = db::open_db()?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    println!("Backfilling entities from {} active memories...", count);

    let mut stmt =
        conn.prepare("SELECT id, title, content FROM memories WHERE status = 'active'")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    print_backfill_progress(&conn, rows, count)
}

fn print_backfill_progress<F>(
    conn: &Connection,
    rows: rusqlite::MappedRows<'_, F>,
    count: i64,
) -> Result<()>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(i64, String, String)>,
{
    let mut total_entities = 0usize;
    let mut memories_processed = 0usize;

    for row in rows {
        let (id, title, content) = row?;
        let entities = crate::entity::extract_entities(&title, &content);
        if !entities.is_empty() {
            crate::entity::link_entities(conn, id, &entities)?;
            total_entities += entities.len();
        }
        memories_processed += 1;
        if memories_processed.is_multiple_of(100) {
            println!("  processed {}/{}", memories_processed, count);
        }
    }

    let unique: i64 = conn
        .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
        .unwrap_or(0);
    println!(
        "Done. {} entities extracted, {} unique entities, {} memories processed.",
        total_entities, unique, memories_processed
    );
    Ok(())
}
