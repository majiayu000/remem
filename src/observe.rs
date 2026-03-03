use anyhow::Result;
use serde::Deserialize;

use crate::db;

/// Tools that produce meaningful observations (modify state or capture research)
const ACTION_TOOLS: &[&str] = &["Write", "Edit", "NotebookEdit", "Bash", "Task"];

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

/// Max tool_response size stored in queue (save DB space)
const MAX_RESPONSE_SIZE: usize = 4000;
/// Larger limit for Task agent results (research/analysis output)
const MAX_TASK_RESPONSE_SIZE: usize = 16000;
/// Task tool_input truncation (keep only prompt core)
const MAX_TASK_INPUT_SIZE: usize = 2000;

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

    // Lightweight cleanup only — no AI calls in hook context.
    // Stale flush (with AI) is handled by summarize_worker instead.
    let stale = db::cleanup_stale_pending(&conn)?;
    if stale > 0 {
        crate::log::info(
            "session-init",
            &format!("cleaned {} stale pending (>1h)", stale),
        );
    }

    timer.done(&format!("project={}", project));
    Ok(())
}

/// PostToolUse hook: queue to SQLite, no AI call.
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

    // Only queue action tools (Write, Edit, Bash, NotebookEdit)
    if !ACTION_TOOLS.contains(&tool_name) {
        crate::log::info("observe", &format!("SKIP tool={} (read-only)", tool_name));
        return Ok(());
    }

    // Filter out routine Bash commands (read-only/build operations)
    if tool_name == "Bash" {
        if let Some(cmd) = hook.tool_input.as_ref().and_then(|v| v["command"].as_str()) {
            if should_skip_bash_command(cmd) {
                crate::log::info(
                    "observe",
                    &format!("SKIP bash cmd={}", db::truncate_str(cmd.trim(), 60)),
                );
                return Ok(());
            }
        }
    }

    let is_task = tool_name == "Task";
    let input_limit = if is_task { MAX_TASK_INPUT_SIZE } else { MAX_RESPONSE_SIZE };
    let response_limit = if is_task { MAX_TASK_RESPONSE_SIZE } else { MAX_RESPONSE_SIZE };

    let tool_input_str = hook.tool_input.as_ref().map(|v| {
        let s = serde_json::to_string(v).unwrap_or_else(|e| {
            crate::log::warn("observe", &format!("tool_input serialize failed: {}", e));
            "{}".to_string()
        });
        if s.len() > input_limit {
            crate::db::truncate_str(&s, input_limit).to_string()
        } else {
            s
        }
    });
    let tool_response_str = hook.tool_response.as_ref().map(|v| {
        let s = serde_json::to_string(v).unwrap_or_else(|e| {
            crate::log::warn("observe", &format!("tool_response serialize failed: {}", e));
            "{}".to_string()
        });
        if s.len() > response_limit {
            crate::db::truncate_str(&s, response_limit).to_string()
        } else {
            s
        }
    });

    let conn = db::open_db()?;
    db::enqueue_pending(
        &conn,
        &session_id,
        &project,
        tool_name,
        tool_input_str.as_deref(),
        tool_response_str.as_deref(),
        Some(cwd),
    )?;
    let job_payload = serde_json::json!({
        "session_id": &session_id,
        "project": &project
    });
    db::enqueue_job(
        &conn,
        db::JobType::Observation,
        &project,
        Some(&session_id),
        &job_payload.to_string(),
        50,
    )?;

    let count = db::count_pending(&conn, &session_id)?;
    crate::log::info(
        "observe",
        &format!(
            "QUEUED tool={} project={} pending={}",
            tool_name, project, count
        ),
    );

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
        assert!(should_skip_bash_command("curl -s http://localhost:9800/tasks/1"));
        assert!(should_skip_bash_command(
            "sleep 60 && curl -s http://localhost:9800/tasks/1"
        ));
    }

    #[test]
    fn keep_mutating_commands() {
        assert!(!should_skip_bash_command("git add src/observe.rs"));
        assert!(!should_skip_bash_command("git commit -m \"feat: tune filter\""));
        assert!(!should_skip_bash_command("git push origin main"));
        assert!(!should_skip_bash_command(
            "curl -X POST http://localhost:9800/tasks"
        ));
    }
}
