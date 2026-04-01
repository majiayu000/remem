use crate::adapter::EventSummary;
use crate::db;
use crate::observe::short_path;

pub(super) fn event_summary(
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
