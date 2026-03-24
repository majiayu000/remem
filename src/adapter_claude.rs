use serde::Deserialize;

use crate::adapter::{EventSummary, ParsedHookEvent, ToolAdapter};
use crate::db;
use crate::observe::short_path;

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

/// Bash command prefixes to skip (routine/read-only operations)
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

pub struct ClaudeCodeAdapter;

impl ToolAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn parse_hook(&self, raw_json: &str) -> Option<ParsedHookEvent> {
        let hook: HookInput = serde_json::from_str(raw_json).ok()?;
        let session_id = hook.session_id?;
        let cwd = hook.cwd;
        let project = db::project_from_cwd(cwd.as_deref().unwrap_or("."));
        Some(ParsedHookEvent {
            session_id,
            cwd,
            project,
            tool_name: hook.tool_name.unwrap_or_else(|| "unknown".into()),
            tool_input: hook.tool_input,
            tool_response: hook.tool_response,
        })
    }

    fn should_skip(&self, event: &ParsedHookEvent) -> bool {
        let name = event.tool_name.as_str();
        SKIP_TOOLS.contains(&name) || !ACTION_TOOLS.contains(&name)
    }

    fn should_skip_bash(&self, command: &str) -> bool {
        should_skip_bash_command(command)
    }

    fn classify_event(&self, event: &ParsedHookEvent) -> Option<EventSummary> {
        event_summary(&event.tool_name, &event.tool_input, &event.tool_response)
    }
}

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

    if cmd_lower.starts_with("sleep ") && cmd_lower.contains("&& curl ") {
        return true;
    }

    false
}

fn event_summary(
    tool_name: &str,
    input: &Option<serde_json::Value>,
    response: &Option<serde_json::Value>,
) -> Option<EventSummary> {
    match tool_name {
        "Edit" => {
            let file = input.as_ref()?.get("file_path")?.as_str()?;
            let files_json = serde_json::to_string(&[file]).ok();
            Some(EventSummary {
                event_type: "file_edit".into(),
                summary: format!("Edit {}", short_path(file)),
                detail: None,
                files_json,
                exit_code: None,
            })
        }
        "Write" => {
            let file = input.as_ref()?.get("file_path")?.as_str()?;
            let files_json = serde_json::to_string(&[file]).ok();
            Some(EventSummary {
                event_type: "file_create".into(),
                summary: format!("Create {}", short_path(file)),
                detail: None,
                files_json,
                exit_code: None,
            })
        }
        "NotebookEdit" => {
            let file = input
                .as_ref()?
                .get("notebook_path")?
                .as_str()
                .or_else(|| input.as_ref()?.get("file_path")?.as_str())?;
            let files_json = serde_json::to_string(&[file]).ok();
            Some(EventSummary {
                event_type: "file_edit".into(),
                summary: format!("NotebookEdit {}", short_path(file)),
                detail: None,
                files_json,
                exit_code: None,
            })
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
            Some(EventSummary {
                event_type: "bash".into(),
                summary: format!("Run `{}` (exit {})", cmd_short, code_str),
                detail: stderr,
                files_json: None,
                exit_code,
            })
        }
        "Grep" => {
            let pattern = input.as_ref()?.get("pattern")?.as_str().unwrap_or("?");
            let path = input
                .as_ref()
                .and_then(|v| v.get("path"))
                .and_then(|p| p.as_str())
                .unwrap_or(".");
            Some(EventSummary {
                event_type: "search".into(),
                summary: format!(
                    "Grep '{}' in {}",
                    db::truncate_str(pattern, 40),
                    short_path(path)
                ),
                detail: None,
                files_json: None,
                exit_code: None,
            })
        }
        "Glob" => {
            let pattern = input.as_ref()?.get("pattern")?.as_str().unwrap_or("?");
            Some(EventSummary {
                event_type: "search".into(),
                summary: format!("Glob {}", pattern),
                detail: None,
                files_json: None,
                exit_code: None,
            })
        }
        "Agent" | "Task" => {
            let desc = input
                .as_ref()
                .and_then(|v| v.get("description").or_else(|| v.get("prompt")))
                .and_then(|d| d.as_str())
                .unwrap_or("agent task");
            Some(EventSummary {
                event_type: "agent".into(),
                summary: format!("Agent: {}", db::truncate_str(desc, 80)),
                detail: None,
                files_json: None,
                exit_code: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn classify_edit_event() {
        let adapter = ClaudeCodeAdapter;
        let event = ParsedHookEvent {
            session_id: "s1".into(),
            cwd: Some("/tmp".into()),
            project: "test".into(),
            tool_name: "Edit".into(),
            tool_input: Some(serde_json::json!({"file_path": "/Users/x/src/main.rs"})),
            tool_response: None,
        };
        let es = adapter.classify_event(&event).unwrap();
        assert_eq!(es.event_type, "file_edit");
        assert!(es.summary.contains("main.rs"));
    }

    #[test]
    fn should_skip_metadata_tools() {
        let adapter = ClaudeCodeAdapter;
        let event = ParsedHookEvent {
            session_id: "s1".into(),
            cwd: None,
            project: "test".into(),
            tool_name: "TodoWrite".into(),
            tool_input: None,
            tool_response: None,
        };
        assert!(adapter.should_skip(&event));
    }
}
