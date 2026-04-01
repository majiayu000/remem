use anyhow::Result;

use crate::{db, memory::Memory};

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
        println!(
            "  [{}] {} | {} | {} | {}",
            memory.id,
            memory.memory_type,
            memory.project,
            created_date(memory.created_at_epoch),
            memory.title
        );
        let preview = preview_text(memory);
        if !preview.is_empty() && preview != memory.title {
            println!("       {}", preview);
        }
    }

    Ok(())
}

pub(super) fn created_date(created_at_epoch: i64) -> String {
    chrono::DateTime::from_timestamp(created_at_epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

pub(super) fn preview_text(memory: &Memory) -> String {
    memory
        .text
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}
