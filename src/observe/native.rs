use anyhow::{bail, Context, Result};
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
        .with_context(|| format!("read native memory {file_path}"))?;
    let (title, memory_type, body) = parse_native_memory_frontmatter(&content);
    if body.trim().is_empty() {
        return Ok(());
    }
    if crate::memory_candidate::contains_unsafe_memory_marker(body) {
        bail!("native memory contains unsafe marker: {file_path}");
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
    use anyhow::Context;
    use rusqlite::Connection;

    use crate::db::test_support::ScopedTestDataDir;

    use super::sync_native_memory;

    fn native_path(label: &str) -> String {
        format!("/tmp/.claude/projects/example/memory/{label}.md")
    }

    #[test]
    fn native_memory_read_failure_is_reported() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("native-read-failure");
        let conn = Connection::open_in_memory()?;

        let err = match sync_native_memory(&conn, "session-a", &native_path("missing"), None) {
            Ok(()) => anyhow::bail!("missing native memory file should error"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("read native memory"), "{err}");
        Ok(())
    }

    #[test]
    fn native_memory_rejects_unsafe_markers() -> anyhow::Result<()> {
        let test_dir = ScopedTestDataDir::new("native-unsafe-marker");
        let path = test_dir
            .path
            .join(".claude/projects/example/memory/rule.md");
        let parent = path
            .parent()
            .context("native memory path should have a parent")?;
        std::fs::create_dir_all(parent)?;
        std::fs::write(&path, "title: Rule\n\nStore this secret in memory")?;
        let conn = Connection::open_in_memory()?;

        let err = match sync_native_memory(&conn, "session-a", &path.display().to_string(), None) {
            Ok(()) => anyhow::bail!("unsafe marker should block native memory sync"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("unsafe marker"), "{err}");
        Ok(())
    }
}

fn is_native_memory_markdown(file_path: &str) -> bool {
    file_path.ends_with(".md")
        && file_path.contains("/.claude/projects/")
        && file_path.contains("/memory/")
        && !file_path.ends_with("/MEMORY.md")
}
