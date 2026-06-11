use serde::Deserialize;

use crate::adapter::{EventSummary, ParsedHookEvent};
use crate::db;
use crate::observe::short_path;

#[cfg(test)]
mod tests;

const ACTION_TOOLS: &[&str] = &[
    "Write",
    "Edit",
    "NotebookEdit",
    "Bash",
    "Grep",
    "Glob",
    "Task",
    "Agent",
];

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
];

const SEARCH_RESPONSE_PREVIEW_BYTES: usize = 240;

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
    let hook: HookInput = match serde_json::from_str(raw_json) {
        Ok(hook) => hook,
        Err(e) => {
            crate::log::error(
                "adapter",
                &format!(
                    "failed to parse hook payload: {e}; raw (truncated): {}",
                    raw_json.chars().take(512).collect::<String>()
                ),
            );
            return None;
        }
    };
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

    if is_search_command(trimmed, &lowered) {
        return !is_bounded_search_command(trimmed, &lowered);
    }

    BASH_SKIP_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
        || lowered.contains("| grep ")
        || is_read_only_polling_cmd(&lowered)
}

fn is_search_tool_input(tool_name: &str, input: &Option<serde_json::Value>) -> bool {
    match tool_name {
        "Grep" | "Glob" => true,
        "Bash" => input
            .as_ref()
            .and_then(|value| value.get("command"))
            .and_then(|command| command.as_str())
            .is_some_and(|command| {
                is_bounded_search_command(command.trim(), &command.trim().to_lowercase())
            }),
        _ => false,
    }
}

fn is_bounded_search_command(trimmed: &str, lowered: &str) -> bool {
    let tokens = shell_like_tokens(trimmed);
    if tokens.is_empty() {
        return false;
    }

    if tokens.first().is_some_and(|token| token == "find") {
        return find_has_target_path(&tokens);
    }

    if lowered.starts_with("git grep ") {
        return true;
    }

    if tokens.first().is_some_and(|token| token == "rg") {
        return search_has_explicit_scope(&tokens[1..], 1);
    }

    if tokens.first().is_some_and(|token| token == "grep") {
        return search_has_explicit_scope(&tokens[1..], 1);
    }

    false
}

fn is_search_command(trimmed: &str, lowered: &str) -> bool {
    let tokens = shell_like_tokens(trimmed);
    tokens
        .first()
        .is_some_and(|token| matches!(token.as_str(), "rg" | "grep" | "find"))
        || lowered.starts_with("git grep ")
}

fn shell_like_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if let Some(open) = quote {
            if ch == open {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }

        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }

        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current.to_lowercase());
                current.clear();
            }
            continue;
        }

        current.push(ch);
    }

    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }

    tokens
}

fn search_has_explicit_scope(tokens: &[String], required_query_terms: usize) -> bool {
    let mut query_terms = 0usize;
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "--" {
            return tokens[index + 1..]
                .iter()
                .any(|candidate| is_scoped_path(candidate));
        }
        if token.starts_with('-') {
            if option_supplies_query(token) {
                query_terms += 1;
                index += 1;
                continue;
            }
            if option_consumes_next(token) {
                if option_consumes_query(token) && index + 1 < tokens.len() {
                    query_terms += 1;
                }
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if query_terms < required_query_terms {
            query_terms += 1;
            index += 1;
            continue;
        }

        if is_scoped_path(token) {
            return true;
        }
        index += 1;
    }
    false
}

fn option_consumes_query(token: &str) -> bool {
    matches!(token, "-e" | "--regexp")
}

fn option_supplies_query(token: &str) -> bool {
    token.starts_with("-e") && token.len() > 2 || token.starts_with("--regexp=")
}

fn option_consumes_next(token: &str) -> bool {
    matches!(
        token,
        "-e" | "--regexp"
            | "-f"
            | "--file"
            | "-g"
            | "--glob"
            | "--type"
            | "-t"
            | "--type-not"
            | "-T"
            | "-m"
            | "--max-count"
            | "-A"
            | "--after-context"
            | "-B"
            | "--before-context"
            | "-C"
            | "--context"
    )
}

fn find_has_target_path(tokens: &[String]) -> bool {
    tokens
        .iter()
        .skip(1)
        .find(|token| !token.starts_with('-') && !find_expression_token(token))
        .is_some_and(|path| is_scoped_path(path))
}

fn find_expression_token(token: &str) -> bool {
    matches!(
        token,
        "!" | "(" | ")" | "-name" | "-iname" | "-path" | "-type" | "-maxdepth" | "-mindepth"
    )
}

fn is_scoped_path(token: &str) -> bool {
    token != "." && token != "/" && token != "~" && !token.starts_with('|')
}

fn redact_and_truncate(text: &str, max_bytes: usize) -> String {
    let redacted = redact_sensitive_text(text);
    db::truncate_str(&redacted, max_bytes).to_string()
}

pub(crate) fn redact_sensitive_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_key(key) {
                        serde_json::Value::String("[REDACTED]".to_string())
                    } else {
                        redact_sensitive_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_sensitive_value).collect::<Vec<_>>())
        }
        serde_json::Value::String(text) => serde_json::Value::String(redact_sensitive_text(text)),
        _ => value.clone(),
    }
}

pub(crate) fn redact_sensitive_text(text: &str) -> String {
    text.lines()
        .map(redact_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_sensitive_line(line: &str) -> String {
    if let Some((prefix, _)) = split_sensitive_assignment(line) {
        return format!("{prefix}[REDACTED]");
    }

    let mut previous_was_bearer = false;
    line.split_whitespace()
        .map(|token| {
            let redacted = if previous_was_bearer {
                "[REDACTED]"
            } else {
                redact_token(token)
            };
            previous_was_bearer = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .eq_ignore_ascii_case("bearer");
            redacted
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_sensitive_assignment(line: &str) -> Option<(&str, &str)> {
    let (idx, separator_len) = line
        .find('=')
        .map(|idx| (idx, 1))
        .or_else(|| line.find(':').map(|idx| (idx, 1)))?;
    let key = line[..idx].trim();
    if !is_sensitive_key(key) {
        return None;
    }
    Some((&line[..idx + separator_len], &line[idx + separator_len..]))
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .to_ascii_lowercase()
        .replace('-', "_");
    matches!(
        normalized.as_str(),
        "api_key"
            | "apikey"
            | "auth"
            | "authorization"
            | "bearer"
            | "cookie"
            | "set_cookie"
            | "password"
            | "passwd"
            | "secret"
            | "token"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "client_secret"
            | "private_key"
    ) || normalized.ends_with("_api_key")
        || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
}

fn redact_token(token: &str) -> &str {
    let trimmed =
        token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
    if contains_prefixed_secret(trimmed)
        || (trimmed.len() >= 32
            && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
            && trimmed.chars().any(|ch| ch.is_ascii_digit()))
    {
        "[REDACTED]"
    } else {
        token
    }
}

fn contains_prefixed_secret(token: &str) -> bool {
    [("sk-", 8), ("ghp_", 8), ("github_pat_", 4), ("xoxb-", 8)]
        .iter()
        .any(|(prefix, min_suffix_len)| {
            contains_prefixed_secret_with(token, prefix, *min_suffix_len)
        })
}

fn contains_prefixed_secret_with(token: &str, prefix: &str, min_suffix_len: usize) -> bool {
    token.match_indices(prefix).any(|(index, _)| {
        has_secret_prefix_boundary(token, index)
            && key_like_suffix_len(&token[index + prefix.len()..]) >= min_suffix_len
    })
}

fn has_secret_prefix_boundary(token: &str, index: usize) -> bool {
    index == 0
        || token[..index]
            .chars()
            .next_back()
            .is_some_and(|ch| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
}

fn key_like_suffix_len(suffix: &str) -> usize {
    suffix
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .count()
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
    let is_search = is_search_tool_input("Bash", input);
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
            .map(|stderr| {
                if is_search {
                    redact_and_truncate(stderr, SEARCH_RESPONSE_PREVIEW_BYTES)
                } else {
                    db::truncate_str(stderr, 500).to_string()
                }
            })
    } else {
        None
    };
    let code_label = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "?".into());
    let event_type = if is_search { "search" } else { "bash" };
    let verb = if is_search { "Search" } else { "Run" };
    let command_label = if is_search {
        redact_and_truncate(command.trim(), 60)
    } else {
        db::truncate_str(command.trim(), 60).to_string()
    };

    Some(EventSummary {
        event_type: event_type.into(),
        summary: format!("{} `{}` (exit {})", verb, command_label, code_label),
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
            redact_and_truncate(pattern, 40),
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
        summary: format!("Glob {}", redact_and_truncate(pattern, 80)),
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
