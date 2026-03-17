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

    crate::log::info("observe", &format!("EVENT {} project={}", summary, project));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::should_skip_bash_command;

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
