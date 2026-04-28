use serde::Deserialize;

use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::db;
use crate::observe::short_path;

const ACTION_TOOLS: &[&str] = &["Write", "Edit", "NotebookEdit", "Bash", "Task", "Agent"];

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
    tool_output: Option<serde_json::Value>,
    tool_result: Option<serde_json::Value>,
}

pub(crate) fn parse_tool_hook(raw_json: &str) -> Option<ParsedHookEvent> {
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
        tool_response: hook.tool_response.or(hook.tool_output).or(hook.tool_result),
    })
}

pub(crate) fn should_skip_tool(tool_name: &str) -> bool {
    SKIP_TOOLS.contains(&tool_name) || !ACTION_TOOLS.contains(&tool_name)
}

pub fn should_skip_bash_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    let lowered = trimmed.to_lowercase();

    BASH_SKIP_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
        || lowered.contains("| grep ")
        || is_read_only_polling_cmd(&lowered)
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

    cmd_lower.starts_with("sleep ") && cmd_lower.contains("&& curl ")
}

pub(crate) fn event_summary(
    tool_name: &str,
    input: &Option<serde_json::Value>,
    response: &Option<serde_json::Value>,
) -> Option<EventSummary> {
    match tool_name {
        "Edit" => file_event(input, "file_path", "file_edit", "Edit "),
        "Write" => file_event(input, "file_path", "file_create", "Create "),
        "NotebookEdit" => notebook_event(input),
        "Bash" => bash_event(input, response),
        "Grep" => grep_event(input),
        "Glob" => glob_event(input),
        "Agent" | "Task" => agent_event(input),
        _ => None,
    }
}

fn file_event(
    input: &Option<serde_json::Value>,
    field: &str,
    event_type: &str,
    prefix: &str,
) -> Option<EventSummary> {
    let file = input.as_ref()?.get(field)?.as_str()?;
    Some(EventSummary {
        event_type: event_type.into(),
        summary: format!("{}{}", prefix, short_path(file)),
        detail: None,
        files_json: serde_json::to_string(&[file]).ok(),
        exit_code: None,
    })
}

fn notebook_event(input: &Option<serde_json::Value>) -> Option<EventSummary> {
    let file = input
        .as_ref()?
        .get("notebook_path")?
        .as_str()
        .or_else(|| input.as_ref()?.get("file_path")?.as_str())?;
    Some(EventSummary {
        event_type: "file_edit".into(),
        summary: format!("NotebookEdit {}", short_path(file)),
        detail: None,
        files_json: serde_json::to_string(&[file]).ok(),
        exit_code: None,
    })
}

fn bash_event(
    input: &Option<serde_json::Value>,
    response: &Option<serde_json::Value>,
) -> Option<EventSummary> {
    let command = input.as_ref()?.get("command")?.as_str()?;
    let exit_code = response
        .as_ref()
        .and_then(|value| value.get("exitCode"))
        .and_then(|code| code.as_i64())
        .map(|code| code as i32);
    let stderr = if exit_code.unwrap_or(0) != 0 {
        response
            .as_ref()
            .and_then(|value| value.get("stderr"))
            .and_then(|stderr| stderr.as_str())
            .map(|stderr| db::truncate_str(stderr, 500).to_string())
    } else {
        None
    };
    let code_label = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "?".into());

    Some(EventSummary {
        event_type: "bash".into(),
        summary: format!(
            "Run `{}` (exit {})",
            db::truncate_str(command.trim(), 60),
            code_label
        ),
        detail: stderr,
        files_json: None,
        exit_code,
    })
}

fn grep_event(input: &Option<serde_json::Value>) -> Option<EventSummary> {
    let pattern = input.as_ref()?.get("pattern")?.as_str().unwrap_or("?");
    let path = input
        .as_ref()
        .and_then(|value| value.get("path"))
        .and_then(|path| path.as_str())
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

fn glob_event(input: &Option<serde_json::Value>) -> Option<EventSummary> {
    let pattern = input.as_ref()?.get("pattern")?.as_str().unwrap_or("?");
    Some(EventSummary {
        event_type: "search".into(),
        summary: format!("Glob {}", pattern),
        detail: None,
        files_json: None,
        exit_code: None,
    })
}

fn agent_event(input: &Option<serde_json::Value>) -> Option<EventSummary> {
    let desc = input
        .as_ref()
        .and_then(|value| value.get("description").or_else(|| value.get("prompt")))
        .and_then(|desc| desc.as_str())
        .unwrap_or("agent task");
    Some(EventSummary {
        event_type: "agent".into(),
        summary: format!("Agent: {}", db::truncate_str(desc, 80)),
        detail: None,
        files_json: None,
        exit_code: None,
    })
}
