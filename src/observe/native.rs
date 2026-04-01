use anyhow::Result;
use rusqlite::Connection;

use super::parse::parse_native_memory_frontmatter;
use super::path::extract_project_from_memory_path;

pub(super) fn sync_native_memory(
    conn: &Connection,
    session_id: &str,
    file_path: &str,
    branch: Option<&str>,
) -> Result<()> {
    if !is_native_memory_markdown(file_path) {
        return Ok(());
    }

    let content = match std::fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(_) => return Ok(()),
    };
    let (title, memory_type, body) = parse_native_memory_frontmatter(&content);
    if body.trim().is_empty() {
        return Ok(());
    }

    let project = extract_project_from_memory_path(file_path);
    let filename = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown");
    let topic_key = format!("native-{}", filename);

    crate::memory::insert_memory_with_branch(
        conn,
        Some(session_id),
        &project,
        Some(&topic_key),
        &title,
        body.trim(),
        &memory_type,
        None,
        branch,
    )?;

    crate::log::info(
        "observe",
        &format!("synced native memory: {} → project={}", filename, project),
    );
    Ok(())
}

fn is_native_memory_markdown(file_path: &str) -> bool {
    file_path.ends_with(".md")
        && file_path.contains("/.claude/projects/")
        && file_path.contains("/memory/")
        && !file_path.ends_with("/MEMORY.md")
}
