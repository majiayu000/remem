use anyhow::Result;

use crate::{db, memory};

pub(in crate::cli) fn run_show(id: i64) -> Result<()> {
    let conn = db::open_db()?;
    let memories = memory::get_memories_by_ids(&conn, &[id], None)?;

    let Some(memory) = memories.first() else {
        println!("Memory {} not found.", id);
        return Ok(());
    };

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
    println!("Created:  {}", format_memory_timestamp(memory.created_at_epoch));
    println!("Updated:  {}", format_memory_timestamp(memory.updated_at_epoch));
    println!();
    println!("{}", memory.text);

    Ok(())
}

pub(super) fn format_memory_timestamp(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}
