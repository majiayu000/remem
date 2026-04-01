use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory;

use super::query_global_preferences;

pub fn list_preferences(conn: &Connection, project: &str) -> Result<()> {
    let project_prefs = memory::get_memories_by_type(conn, project, "preference", 50)?;
    let global_prefs = query_global_preferences(conn, 10).unwrap_or_default();

    if project_prefs.is_empty() && global_prefs.is_empty() {
        println!("No preferences found.");
        return Ok(());
    }

    if !project_prefs.is_empty() {
        println!("Project preferences ({}):", project);
        for pref in &project_prefs {
            let text_preview: String = pref.text.chars().take(80).collect();
            println!("  [{}] {}", pref.id, text_preview);
        }
    }

    if !global_prefs.is_empty() {
        println!("\nGlobal preferences (3+ projects):");
        for pref in &global_prefs {
            let text_preview: String = pref.text.chars().take(80).collect();
            println!("  [{}] {} (from: {})", pref.id, text_preview, pref.project);
        }
    }

    Ok(())
}

pub fn add_preference(conn: &Connection, project: &str, text: &str, global: bool) -> Result<i64> {
    let title = format!("Preference: {}", &text[..text.len().min(60)]);
    let topic_key = format!(
        "manual-preference-{}",
        crate::memory::slugify_for_topic(text, 50)
    );
    let scope = if global { "global" } else { "project" };
    memory::insert_memory_full(
        conn,
        None,
        project,
        Some(&topic_key),
        &title,
        text,
        "preference",
        None,
        None,
        scope,
        None,
    )
}

pub fn remove_preference(conn: &Connection, id: i64) -> Result<bool> {
    let count = conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1 AND memory_type = 'preference'",
        params![id],
    )?;
    Ok(count > 0)
}
