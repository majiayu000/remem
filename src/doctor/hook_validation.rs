use serde_json::Value;
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) struct ExpectedHookCommand<'a> {
    pub executable: &'a Path,
    pub subcommand: &'static str,
    pub host: &'static str,
}

pub(super) fn expected_hook_events(host: &str) -> &'static [&'static str] {
    match host {
        "codex" => &["SessionStart", "Stop"],
        _ => &[
            "PostToolUse",
            "PreCompact",
            "Stop",
            "SessionStart",
            "UserPromptSubmit",
        ],
    }
}

pub(super) fn runtime_host(host: &str) -> &'static str {
    match host {
        "codex" => "codex-cli",
        _ => "claude-code",
    }
}

pub(super) fn expected_hook_command<'a>(
    host: &str,
    event: &str,
    executable: &'a Path,
) -> Option<ExpectedHookCommand<'a>> {
    let subcommand = expected_subcommand(event)?;
    Some(ExpectedHookCommand {
        executable,
        subcommand,
        host: runtime_host(host),
    })
}

pub(super) fn expected_hook_executable_from_hooks(doc: &Value, host: &str) -> Option<String> {
    let mut paths = Vec::new();
    for event in expected_hook_events(host) {
        let Some(subcommand) = expected_subcommand(event) else {
            continue;
        };
        for command in hook_commands_for_event(doc, event) {
            let Some(invocation) = parse_remem_invocation(command) else {
                continue;
            };
            if invocation.subcommand.as_deref() == Some(subcommand)
                && invocation.resolved_host() == Some(runtime_host(host))
                && !paths.contains(&invocation.executable)
            {
                paths.push(invocation.executable);
            }
        }
    }
    match paths.as_slice() {
        [path] => Some(path.clone()),
        _ => None,
    }
}

pub(super) fn event_has_expected_remem_hook(
    doc: &Value,
    event: &str,
    expected: ExpectedHookCommand<'_>,
) -> bool {
    hook_commands_for_event(doc, event).any(|command| {
        parse_remem_invocation(command).is_some_and(|invocation| {
            Path::new(&invocation.executable) == expected.executable
                && invocation.subcommand.as_deref() == Some(expected.subcommand)
                && invocation.resolved_host() == Some(expected.host)
        })
    })
}

pub(super) fn event_has_remem_subcommand_hook(doc: &Value, event: &str, subcommand: &str) -> bool {
    hook_commands_for_event(doc, event).any(|command| {
        parse_remem_invocation(command)
            .is_some_and(|invocation| invocation.subcommand.as_deref() == Some(subcommand))
    })
}

pub(super) fn hook_command_strings(doc: &Value) -> impl Iterator<Item = &str> {
    doc.get("hooks")
        .and_then(|hooks| hooks.as_object())
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(|entries| entries.as_array())
        .flatten()
        .filter_map(|entry| entry.get("hooks").and_then(|hooks| hooks.as_array()))
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(|command| command.as_str()))
}

pub(super) fn extract_remem_command_path(command: &str) -> Option<String> {
    parse_remem_invocation(command).map(|invocation| invocation.executable)
}

fn hook_commands_for_event<'a>(doc: &'a Value, event: &str) -> impl Iterator<Item = &'a str> {
    doc.get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(|entries| entries.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("hooks").and_then(|hooks| hooks.as_array()))
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(|command| command.as_str()))
}

#[derive(Debug, PartialEq, Eq)]
struct RememInvocation {
    executable: String,
    subcommand: Option<String>,
    host: Option<String>,
    env_host: Option<String>,
}

impl RememInvocation {
    fn resolved_host(&self) -> Option<&str> {
        self.host.as_deref().or(self.env_host.as_deref())
    }
}

fn parse_remem_invocation(command: &str) -> Option<RememInvocation> {
    let tokens = shell_words(command)?;
    let command_index = tokens.iter().position(|token| !is_env_assignment(token))?;
    if !is_remem_command_token(&tokens[command_index]) {
        return None;
    }
    let host = find_host_arg(&tokens[command_index + 1..]);

    Some(RememInvocation {
        executable: tokens[command_index].clone(),
        subcommand: tokens.get(command_index + 1).cloned(),
        host,
        env_host: find_legacy_host_env(&tokens[..command_index]),
    })
}

fn expected_subcommand(event: &str) -> Option<&'static str> {
    match event {
        "PostToolUse" => Some("observe"),
        "PreCompact" | "Stop" => Some("summarize"),
        "SessionStart" => Some("context"),
        "UserPromptSubmit" => Some("session-init"),
        _ => None,
    }
}

fn find_host_arg(tokens: &[String]) -> Option<String> {
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        if token == "--host" {
            return iter
                .next()
                .map(|host| crate::runtime_config::normalize_host(host));
        }
        if let Some(host) = token.strip_prefix("--host=") {
            return Some(crate::runtime_config::normalize_host(host));
        }
    }
    None
}

fn find_legacy_host_env(tokens: &[String]) -> Option<String> {
    for token in tokens {
        let Some((name, value)) = env_assignment(token) else {
            continue;
        };
        if matches!(name, "REMEM_HOOK_HOST" | "REMEM_CONTEXT_HOST") {
            return Some(crate::runtime_config::normalize_host(value));
        }
    }
    for token in tokens {
        let Some((name, value)) = env_assignment(token) else {
            continue;
        };
        if matches!(name, "REMEM_SUMMARY_EXECUTOR" | "REMEM_EXECUTOR") {
            match value.trim().to_ascii_lowercase().as_str() {
                "codex" | "codex-cli" => {
                    return Some(crate::runtime_config::CODEX_HOST.to_string())
                }
                "claude" | "claude-cli" | "cli" => {
                    return Some(crate::runtime_config::CLAUDE_HOST.to_string());
                }
                _ => {}
            }
        }
    }
    None
}

fn is_remem_command_token(token: &str) -> bool {
    Path::new(token)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "remem")
}

fn is_env_assignment(token: &str) -> bool {
    env_assignment(token).is_some()
}

fn env_assignment(token: &str) -> Option<(&str, &str)> {
    let (name, value) = token.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Some((name, value))
    } else {
        None
    }
}

fn shell_words(command: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = Quote::None;
    let mut in_token = false;

    while let Some(ch) = chars.next() {
        match quote {
            Quote::None => match ch {
                '\'' => {
                    quote = Quote::Single;
                    in_token = true;
                }
                '"' => {
                    quote = Quote::Double;
                    in_token = true;
                }
                '\\' => {
                    in_token = true;
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ch if ch.is_whitespace() => {
                    if in_token {
                        tokens.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                _ => {
                    current.push(ch);
                    in_token = true;
                }
            },
            Quote::Single => {
                if ch == '\'' {
                    quote = Quote::None;
                } else {
                    current.push(ch);
                }
            }
            Quote::Double => match ch {
                '"' => quote = Quote::None,
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                _ => current.push(ch),
            },
        }
    }

    if quote != Quote::None {
        return None;
    }
    if in_token {
        tokens.push(current);
    }
    Some(tokens)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remem_invocation_accepts_env_prefix_and_quotes() {
        let invocation = parse_remem_invocation(
            "REMEM_CONTEXT_HOST=codex-cli '/opt/remem bin/remem' context --host codex",
        )
        .expect("remem invocation should parse");

        assert_eq!(
            invocation,
            RememInvocation {
                executable: "/opt/remem bin/remem".to_string(),
                subcommand: Some("context".to_string()),
                host: Some("codex-cli".to_string()),
                env_host: Some("codex-cli".to_string()),
            }
        );
    }

    #[test]
    fn parse_remem_invocation_handles_single_quote_escaping() {
        let invocation = parse_remem_invocation("'/tmp/remem'\\''bin/remem' observe --host=claude")
            .expect("remem invocation should parse");

        assert_eq!(invocation.executable, "/tmp/remem'bin/remem");
        assert_eq!(invocation.subcommand.as_deref(), Some("observe"));
        assert_eq!(invocation.host.as_deref(), Some("claude-code"));
    }

    #[test]
    fn parse_remem_invocation_rejects_text_without_remem_executable() {
        assert!(parse_remem_invocation("NOTE=remem echo ok").is_none());
        assert!(parse_remem_invocation("echo remem context --host codex-cli").is_none());
        assert!(parse_remem_invocation("/bin/sh -c 'remem context --host codex-cli'").is_none());
    }

    #[test]
    fn expected_hook_requires_exact_executable_path() {
        let doc = serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "command": "/wrong/stale/remem context --host codex-cli"
                    }]
                }]
            }
        });
        let Some(expected) =
            expected_hook_command("codex", "SessionStart", Path::new("/expected/bin/remem"))
        else {
            panic!("known hook event should build expected command");
        };

        assert!(!event_has_expected_remem_hook(
            &doc,
            "SessionStart",
            expected
        ));
    }

    #[test]
    fn expected_hook_accepts_legacy_env_host() {
        let doc = serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "command": "REMEM_CONTEXT_HOST=codex-cli /tmp/remem context"
                    }]
                }]
            }
        });
        let Some(expected) =
            expected_hook_command("codex", "SessionStart", Path::new("/tmp/remem"))
        else {
            panic!("known hook event should build expected command");
        };

        assert!(event_has_expected_remem_hook(
            &doc,
            "SessionStart",
            expected
        ));
    }
}
