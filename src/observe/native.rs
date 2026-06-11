use anyhow::{Context, Result};
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

    let content = std::fs::read_to_string(file_path)
        .with_context(|| format!("read native memory file {}", file_path))?;
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

#[cfg(test)]
mod tests {
    use super::sync_native_memory;
    use crate::db::{self, test_support::ScopedTestDataDir};

    #[test]
    fn sync_native_memory_ignores_non_native_path() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("native-non-memory");
        let conn = db::open_db()?;

        sync_native_memory(&conn, "sess", "/tmp/remem/notes.md", None)?;
        Ok(())
    }

    #[test]
    fn sync_native_memory_reports_native_read_error() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("native-read-error");
        let conn = db::open_db()?;
        let err = sync_native_memory(
            &conn,
            "sess",
            "/tmp/.claude/projects/remem/memory/missing.md",
            None,
        )
        .expect_err("missing native memory file should error");

        assert!(err.to_string().contains("read native memory file"));
        Ok(())
    }
}

fn is_native_memory_markdown(file_path: &str) -> bool {
    file_path.ends_with(".md")
        && file_path.contains("/.claude/projects/")
        && file_path.contains("/memory/")
        && !file_path.ends_with("/MEMORY.md")
}
