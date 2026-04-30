use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::query_global_preferences;

pub fn dedup_with_claude_md(prefs: &[Memory], cwd: &str) -> Vec<usize> {
    let claude_md_path = std::path::Path::new(cwd).join("CLAUDE.md");
    let claude_md_content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();

    if claude_md_content.is_empty() {
        return (0..prefs.len()).collect();
    }

    let claude_lower = claude_md_content.to_lowercase();
    (0..prefs.len())
        .filter(|&i| {
            let title_lower = prefs[i].title.to_lowercase();
            let search_term = title_lower
                .strip_prefix("preference: ")
                .unwrap_or(&title_lower);
            !claude_lower.contains(search_term)
        })
        .collect()
}

pub fn render_preferences(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
) -> Result<()> {
    render_preferences_with_limits(output, conn, project, cwd, 20, 10, 1500).map(|_| ())
}

pub fn render_preferences_with_limits(
    output: &mut String,
    conn: &Connection,
    project: &str,
    cwd: &str,
    project_limit: usize,
    global_limit: usize,
    char_limit: usize,
) -> Result<usize> {
    let project_prefs =
        memory::get_memories_by_type(conn, project, "preference", project_limit as i64)?;
    let global_prefs = query_global_preferences(conn, global_limit).unwrap_or_default();

    let mut all_prefs = project_prefs;
    let project_topics: std::collections::HashSet<String> = all_prefs
        .iter()
        .filter_map(|memory| memory.topic_key.clone())
        .collect();
    for global_pref in global_prefs {
        if let Some(ref topic_key) = global_pref.topic_key {
            if !project_topics.contains(topic_key) {
                all_prefs.push(global_pref);
            }
        }
    }

    if all_prefs.is_empty() {
        return Ok(0);
    }

    let keep_indices = dedup_with_claude_md(&all_prefs, cwd);
    if keep_indices.is_empty() {
        return Ok(0);
    }

    output.push_str("## Your Preferences (always apply these)\n");
    let mut total_chars = 0;
    let mut rendered = 0usize;

    for &idx in &keep_indices {
        let pref = &all_prefs[idx];
        let text = pref.text.trim();
        let preview: String = text.chars().take(120).collect();
        let line = if preview.len() < text.len() {
            format!("- {}\n", preview)
        } else {
            format!("- {}\n", text)
        };
        if total_chars + line.len() > char_limit && total_chars > 0 {
            break;
        }
        output.push_str(&line);
        total_chars += line.len();
        rendered += 1;
    }
    output.push('\n');

    Ok(rendered)
}
