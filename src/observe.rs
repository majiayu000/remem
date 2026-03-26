use anyhow::Result;

use crate::db;
use crate::memory;

/// Shorten a file path to last 2 components for compact display.
pub fn short_path(full: &str) -> &str {
    let parts: Vec<&str> = full.rsplitn(3, '/').collect();
    match parts.len() {
        1 => parts[0],
        2 => full,
        _ => {
            let start = full.len() - parts[0].len() - parts[1].len() - 1;
            &full[start..]
        }
    }
}

pub async fn session_init() -> Result<()> {
    let timer = crate::log::Timer::start("session-init", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(&input, 500)),
    );

    let Some((_adapter, event)) = crate::adapter::detect_adapter(&input) else {
        anyhow::bail!("no adapter matched session_init input");
    };

    crate::log::info(
        "session-init",
        &format!("project={} session={}", event.project, event.session_id),
    );

    let conn = db::open_db()?;
    db::upsert_session(&conn, &event.session_id, &event.project, None)?;

    timer.done(&format!("project={}", event.project));
    Ok(())
}

/// PostToolUse hook: classify event via adapter, write to SQLite, enqueue for LLM.
pub async fn observe() -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;

    let Some((adapter, event)) = crate::adapter::detect_adapter(&input) else {
        return Ok(());
    };

    if adapter.should_skip(&event) {
        return Ok(());
    }

    // Bash-specific filter
    if event.tool_name == "Bash" {
        if let Some(cmd) = event
            .tool_input
            .as_ref()
            .and_then(|v| v["command"].as_str())
        {
            if adapter.should_skip_bash(cmd) {
                return Ok(());
            }
        }
    }

    let Some(es) = adapter.classify_event(&event) else {
        return Ok(());
    };

    let branch = event.cwd.as_deref().and_then(db::detect_git_branch);
    let _commit_sha = event.cwd.as_deref().and_then(db::detect_git_commit);

    let conn = db::open_db()?;
    memory::insert_event(
        &conn,
        &event.session_id,
        &event.project,
        &es.event_type,
        &es.summary,
        es.detail.as_deref(),
        es.files_json.as_deref(),
        es.exit_code,
    )?;

    let tool_input_str = event.tool_input.as_ref().map(|v| v.to_string());
    let tool_response_str = event.tool_response.as_ref().map(|v| v.to_string());
    db::enqueue_pending(
        &conn,
        &event.session_id,
        &event.project,
        &event.tool_name,
        tool_input_str.as_deref(),
        tool_response_str.as_deref(),
        event.cwd.as_deref(),
    )?;

    crate::log::info(
        "observe",
        &format!(
            "EVENT {} project={} branch={:?}",
            es.summary, event.project, branch
        ),
    );

    // Sync native Claude Code memory writes to remem DB
    if matches!(event.tool_name.as_str(), "Write" | "Edit") {
        if let Some(file_path) = event
            .tool_input
            .as_ref()
            .and_then(|v| v["file_path"].as_str())
        {
            if let Err(e) =
                sync_native_memory(&conn, &event.session_id, file_path, branch.as_deref())
            {
                crate::log::warn("observe", &format!("native memory sync failed: {}", e));
            }
        }
    }

    Ok(())
}

/// Detect writes to Claude Code native memory files and sync to remem DB.
fn sync_native_memory(
    conn: &rusqlite::Connection,
    session_id: &str,
    file_path: &str,
    branch: Option<&str>,
) -> Result<()> {
    if !file_path.ends_with(".md") {
        return Ok(());
    }
    if !file_path.contains("/.claude/projects/") || !file_path.contains("/memory/") {
        return Ok(());
    }
    if file_path.ends_with("/MEMORY.md") {
        return Ok(());
    }

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let (title, memory_type, body) = parse_native_memory_frontmatter(&content);

    if body.trim().is_empty() {
        return Ok(());
    }

    let project = extract_project_from_memory_path(file_path);

    let filename = std::path::Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let topic_key = format!("native-{}", filename);

    memory::insert_memory_with_branch(
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

fn parse_native_memory_frontmatter(content: &str) -> (String, String, &str) {
    let default_title = "Untitled memory".to_string();
    let default_type = "discovery".to_string();

    if !content.starts_with("---") {
        return (default_title, default_type, content);
    }

    let after_first = &content[3..];
    let Some(end_pos) = after_first.find("\n---") else {
        return (default_title, default_type, content);
    };

    let frontmatter = &after_first[..end_pos];
    let body_start = 3 + end_pos + 4;
    let body = if body_start < content.len() {
        &content[body_start..]
    } else {
        ""
    };

    let mut name = None;
    let mut mem_type = None;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("type:") {
            let raw = val.trim();
            mem_type = Some(match raw {
                "user" | "feedback" => "preference".to_string(),
                "project" => "discovery".to_string(),
                "reference" => "discovery".to_string(),
                other => other.to_string(),
            });
        }
    }

    (
        name.unwrap_or(default_title),
        mem_type.unwrap_or(default_type),
        body,
    )
}

fn extract_project_from_memory_path(file_path: &str) -> String {
    let Some(projects_pos) = file_path.find("/projects/") else {
        return "unknown".to_string();
    };
    let after_projects = &file_path[projects_pos + "/projects/".len()..];
    let slug = after_projects.split('/').next().unwrap_or("");
    if slug.is_empty() {
        return "unknown".to_string();
    }
    let mut decoded = slug.replace('-', "/");
    if !decoded.starts_with('/') {
        decoded = format!("/{decoded}");
    }
    crate::project_id::canonical_project_path(&decoded)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_full() {
        let content =
            "---\nname: my memory\ndescription: test\ntype: feedback\n---\nBody content here.";
        let (title, mem_type, body) = parse_native_memory_frontmatter(content);
        assert_eq!(title, "my memory");
        assert_eq!(mem_type, "preference");
        assert_eq!(body.trim(), "Body content here.");
    }

    #[test]
    fn parse_frontmatter_missing() {
        let content = "Just plain text, no frontmatter.";
        let (title, mem_type, body) = parse_native_memory_frontmatter(content);
        assert_eq!(title, "Untitled memory");
        assert_eq!(mem_type, "discovery");
        assert_eq!(body, content);
    }

    #[test]
    fn parse_frontmatter_project_type() {
        let content = "---\nname: deploy notes\ntype: project\n---\nContent.";
        let (_, mem_type, _) = parse_native_memory_frontmatter(content);
        assert_eq!(mem_type, "discovery");
    }

    #[test]
    fn extract_project_from_path() {
        let path = "/Users/lifcc/.claude/projects/-Users-lifcc-Desktop-code-AI-tools-remem/memory/feedback_quality.md";
        let project = extract_project_from_memory_path(path);
        assert_eq!(project, "/Users/lifcc/Desktop/code/AI/tools/remem");
    }

    #[test]
    fn extract_project_short_slug() {
        let path = "/Users/x/.claude/projects/-myproject/memory/foo.md";
        let project = extract_project_from_memory_path(path);
        assert_eq!(project, "/myproject");
    }
}
