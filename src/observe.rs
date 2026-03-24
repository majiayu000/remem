use anyhow::Result;
use serde::Deserialize;

use crate::db;
use crate::memory;

/// Tools that produce meaningful events (modify state or capture research)
const ACTION_TOOLS: &[&str] = &["Write", "Edit", "NotebookEdit", "Bash", "Task", "Agent"];

/// Tools to always skip (metadata/navigation)
const SKIP_TOOLS: &[&str] = &[
    "ListMcpResourcesTool",
    "SlashCommand",
    "Skill",
    "TodoWrite",
    "AskUserQuestion",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "EnterPlanMode",
    "ExitPlanMode",
];

/// Bash command prefixes to skip (routine/read-only operations, not worth recording)
const BASH_SKIP_PREFIXES: &[&str] = &[
    "git status",
    "git log",
    "git diff",
    "git branch",
    "git stash list",
    "git remote",
    "git fetch",
    "git show",
    "ls",
    "pwd",
    "echo ",
    "which ",
    "type ",
    "whereis ",
    "cat ",
    "head ",
    "tail ",
    "wc ",
    "file ",
    "npm install",
    "npm ci",
    "yarn install",
    "pnpm install",
    "cargo build",
    "cargo check",
    "cargo clippy",
    "cargo fmt",
    "cd ",
    "pushd ",
    "popd",
    "lsof ",
    "ps ",
    "top",
    "htop",
    "df ",
    "du ",
    "grep ",
    "rg ",
    "find ",
    "git grep",
];

#[derive(Debug, Deserialize)]
struct HookInput {
    session_id: Option<String>,
    cwd: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_response: Option<serde_json::Value>,
}

use crate::db::project_from_cwd;

pub fn should_skip_bash_command(cmd: &str) -> bool {
    let cmd_trimmed = cmd.trim();
    let cmd_lower = cmd_trimmed.to_lowercase();

    BASH_SKIP_PREFIXES
        .iter()
        .any(|prefix| cmd_lower.starts_with(prefix))
        || cmd_lower.contains("| grep ")
        || is_read_only_polling_cmd(&cmd_lower)
}

fn is_read_only_polling_cmd(cmd_lower: &str) -> bool {
    let is_curl = cmd_lower.starts_with("curl ");
    let has_mutation_method = cmd_lower.contains("-x post")
        || cmd_lower.contains("-x put")
        || cmd_lower.contains("-x patch")
        || cmd_lower.contains("-x delete")
        || cmd_lower.contains("--request post")
        || cmd_lower.contains("--request put")
        || cmd_lower.contains("--request patch")
        || cmd_lower.contains("--request delete");

    if is_curl && !has_mutation_method {
        return true;
    }

    // Common status polling pattern: sleep N && curl ...
    if cmd_lower.starts_with("sleep ") && cmd_lower.contains("&& curl ") {
        return true;
    }

    false
}

pub async fn session_init() -> Result<()> {
    let timer = crate::log::Timer::start("session-init", "");
    let input = std::io::read_to_string(std::io::stdin())?;
    crate::log::debug(
        "session-init",
        &format!("raw input: {}", crate::db::truncate_str(&input, 500)),
    );
    let hook: HookInput = serde_json::from_str(&input)?;

    let session_id = hook
        .session_id
        .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);

    crate::log::info(
        "session-init",
        &format!("project={} session={}", project, session_id),
    );

    let conn = db::open_db()?;
    db::upsert_session(&conn, &session_id, &project, None)?;

    timer.done(&format!("project={}", project));
    Ok(())
}

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

/// Generate a structured event from a PostToolUse hook.
/// Returns (event_type, summary, detail, files_json) or None to skip.
fn event_summary(
    tool_name: &str,
    input: &Option<serde_json::Value>,
    response: &Option<serde_json::Value>,
) -> Option<(String, String, Option<String>, Option<String>, Option<i32>)> {
    match tool_name {
        "Edit" => {
            let file = input.as_ref()?.get("file_path")?.as_str()?;
            let files_json = serde_json::to_string(&[file]).ok();
            Some((
                "file_edit".into(),
                format!("Edit {}", short_path(file)),
                None,
                files_json,
                None,
            ))
        }
        "Write" => {
            let file = input.as_ref()?.get("file_path")?.as_str()?;
            let files_json = serde_json::to_string(&[file]).ok();
            Some((
                "file_create".into(),
                format!("Create {}", short_path(file)),
                None,
                files_json,
                None,
            ))
        }
        "NotebookEdit" => {
            let file = input
                .as_ref()?
                .get("notebook_path")?
                .as_str()
                .or_else(|| input.as_ref()?.get("file_path")?.as_str())?;
            let files_json = serde_json::to_string(&[file]).ok();
            Some((
                "file_edit".into(),
                format!("NotebookEdit {}", short_path(file)),
                None,
                files_json,
                None,
            ))
        }
        "Bash" => {
            let cmd = input.as_ref()?.get("command")?.as_str()?;
            let cmd_short = db::truncate_str(cmd.trim(), 60);
            let exit_code = response
                .as_ref()
                .and_then(|r| r.get("exitCode"))
                .and_then(|c| c.as_i64())
                .map(|c| c as i32);
            let code_str = exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into());
            let stderr = if exit_code.unwrap_or(0) != 0 {
                response
                    .as_ref()
                    .and_then(|r| r.get("stderr"))
                    .and_then(|s| s.as_str())
                    .map(|s| db::truncate_str(s, 500).to_string())
            } else {
                None
            };
            Some((
                "bash".into(),
                format!("Run `{}` (exit {})", cmd_short, code_str),
                stderr,
                None,
                exit_code,
            ))
        }
        "Grep" => {
            let pattern = input.as_ref()?.get("pattern")?.as_str().unwrap_or("?");
            let path = input
                .as_ref()
                .and_then(|v| v.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or(".");
            Some((
                "search".into(),
                format!(
                    "Grep '{}' in {}",
                    db::truncate_str(pattern, 40),
                    short_path(path)
                ),
                None,
                None,
                None,
            ))
        }
        "Glob" => {
            let pattern = input.as_ref()?.get("pattern")?.as_str().unwrap_or("?");
            Some((
                "search".into(),
                format!("Glob {}", pattern),
                None,
                None,
                None,
            ))
        }
        "Agent" | "Task" => {
            let desc = input
                .as_ref()
                .and_then(|v| v.get("description").or_else(|| v.get("prompt")))
                .and_then(|d| d.as_str())
                .unwrap_or("agent task");
            Some((
                "agent".into(),
                format!("Agent: {}", db::truncate_str(desc, 80)),
                None,
                None,
                None,
            ))
        }
        _ => None,
    }
}

/// PostToolUse hook: write event directly to SQLite (zero LLM, rule-based).
pub async fn observe() -> Result<()> {
    let input = std::io::read_to_string(std::io::stdin())?;
    let hook: HookInput = serde_json::from_str(&input)?;

    let session_id = hook
        .session_id
        .ok_or_else(|| anyhow::anyhow!("missing session_id"))?;
    let cwd = hook.cwd.as_deref().unwrap_or(".");
    let project = project_from_cwd(cwd);
    let tool_name = hook.tool_name.as_deref().unwrap_or("unknown");

    // Skip metadata tools
    if SKIP_TOOLS.contains(&tool_name) {
        return Ok(());
    }

    // Only record action tools
    if !ACTION_TOOLS.contains(&tool_name) {
        return Ok(());
    }

    // Filter out routine Bash commands
    if tool_name == "Bash" {
        if let Some(cmd) = hook.tool_input.as_ref().and_then(|v| v["command"].as_str()) {
            if should_skip_bash_command(cmd) {
                return Ok(());
            }
        }
    }

    // Generate structured event summary (rule-based, zero LLM)
    let Some((event_type, summary, detail, files_json, exit_code)) =
        event_summary(tool_name, &hook.tool_input, &hook.tool_response)
    else {
        return Ok(());
    };

    // Detect git branch/commit from working directory
    let branch = hook.cwd.as_deref().and_then(db::detect_git_branch);
    let _commit_sha = hook.cwd.as_deref().and_then(db::detect_git_commit);

    let conn = db::open_db()?;
    memory::insert_event(
        &conn,
        &session_id,
        &project,
        &event_type,
        &summary,
        detail.as_deref(),
        files_json.as_deref(),
        exit_code,
    )?;

    // Enqueue for LLM extraction
    let tool_input_str = hook.tool_input.as_ref().map(|v| v.to_string());
    let tool_response_str = hook.tool_response.as_ref().map(|v| v.to_string());
    db::enqueue_pending(
        &conn,
        &session_id,
        &project,
        tool_name,
        tool_input_str.as_deref(),
        tool_response_str.as_deref(),
        hook.cwd.as_deref(),
    )?;

    crate::log::info(
        "observe",
        &format!("EVENT {} project={} branch={:?}", summary, project, branch),
    );

    // Sync native Claude Code memory writes to remem DB
    if matches!(tool_name, "Write" | "Edit") {
        if let Some(file_path) = hook
            .tool_input
            .as_ref()
            .and_then(|v| v["file_path"].as_str())
        {
            if let Err(e) = sync_native_memory(&conn, &session_id, file_path, branch.as_deref()) {
                crate::log::warn("observe", &format!("native memory sync failed: {}", e));
            }
        }
    }

    Ok(())
}

/// Detect writes to Claude Code native memory files and sync to remem DB.
/// Pattern: ~/.claude/projects/*/memory/*.md (excluding MEMORY.md index)
fn sync_native_memory(
    conn: &rusqlite::Connection,
    session_id: &str,
    file_path: &str,
    branch: Option<&str>,
) -> Result<()> {
    // Must be a .md file in a Claude Code memory directory
    if !file_path.ends_with(".md") {
        return Ok(());
    }
    if !file_path.contains("/.claude/projects/") || !file_path.contains("/memory/") {
        return Ok(());
    }
    // Skip MEMORY.md index file
    if file_path.ends_with("/MEMORY.md") {
        return Ok(());
    }

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // file might not exist yet during Edit
    };

    // Parse frontmatter: ---\nkey: value\n---\ncontent
    let (title, memory_type, body) = parse_native_memory_frontmatter(&content);

    if body.trim().is_empty() {
        return Ok(());
    }

    // Extract project from the path: ~/.claude/projects/-Users-x-code-AI-tools-remem/memory/
    let project = extract_project_from_memory_path(file_path);

    // Use filename (without .md) as topic_key for UPSERT
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

/// Parse Claude Code memory frontmatter format.
/// Returns (title, memory_type, body).
fn parse_native_memory_frontmatter(content: &str) -> (String, String, &str) {
    let default_title = "Untitled memory".to_string();
    let default_type = "discovery".to_string();

    // Check for frontmatter delimiters
    if !content.starts_with("---") {
        return (default_title, default_type, content);
    }

    let after_first = &content[3..];
    let Some(end_pos) = after_first.find("\n---") else {
        return (default_title, default_type, content);
    };

    let frontmatter = &after_first[..end_pos];
    let body_start = 3 + end_pos + 4; // skip "---" + frontmatter + "\n---"
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
            // Map Claude Code types to remem types
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

/// Extract project name from Claude Code memory path.
/// ~/.claude/projects/-Users-x-Desktop-code-AI-tools-remem/memory/foo.md
/// → "tools/remem" (last 2 meaningful components)
fn extract_project_from_memory_path(file_path: &str) -> String {
    // Find the project slug between /projects/ and /memory/
    let Some(projects_pos) = file_path.find("/projects/") else {
        return "unknown".to_string();
    };
    let after_projects = &file_path[projects_pos + "/projects/".len()..];
    let slug = after_projects.split('/').next().unwrap_or("unknown");

    // The slug is like "-Users-x-Desktop-code-AI-tools-remem"
    // Convert back to path components and take last 2
    let parts: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else if parts.len() == 1 {
        parts[0].to_string()
    } else {
        "unknown".to_string()
    }
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
        assert_eq!(mem_type, "preference"); // feedback → preference
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
        assert_eq!(mem_type, "discovery"); // project → discovery
    }

    #[test]
    fn extract_project_from_path() {
        let path = "/Users/lifcc/.claude/projects/-Users-lifcc-Desktop-code-AI-tools-remem/memory/feedback_quality.md";
        let project = extract_project_from_memory_path(path);
        assert_eq!(project, "tools/remem");
    }

    #[test]
    fn extract_project_short_slug() {
        let path = "/Users/x/.claude/projects/-myproject/memory/foo.md";
        let project = extract_project_from_memory_path(path);
        assert_eq!(project, "myproject");
    }

    #[test]
    fn skip_read_only_search_commands() {
        assert!(should_skip_bash_command("grep -rn \"foo\" src/"));
        assert!(should_skip_bash_command("rg -n foo src"));
        assert!(should_skip_bash_command("find src -name '*.ts'"));
        assert!(should_skip_bash_command("git grep -n startIngestionJob"));
    }

    #[test]
    fn skip_read_only_polling_commands() {
        assert!(should_skip_bash_command(
            "curl -s http://localhost:9800/tasks/1"
        ));
        assert!(should_skip_bash_command(
            "sleep 60 && curl -s http://localhost:9800/tasks/1"
        ));
    }

    #[test]
    fn keep_mutating_commands() {
        assert!(!should_skip_bash_command("git add src/observe.rs"));
        assert!(!should_skip_bash_command(
            "git commit -m \"feat: tune filter\""
        ));
        assert!(!should_skip_bash_command("git push origin main"));
        assert!(!should_skip_bash_command(
            "curl -X POST http://localhost:9800/tasks"
        ));
    }
}
