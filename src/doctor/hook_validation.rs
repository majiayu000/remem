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
    let subcommand = match event {
        "PostToolUse" => "observe",
        "PreCompact" | "Stop" => "summarize",
        "SessionStart" => "context",
        "UserPromptSubmit" => "session-init",
        _ => return None,
    };
    Some(ExpectedHookCommand {
        executable,
        subcommand,
        host: runtime_host(host),
    })
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
                && invocation.host.as_deref() == Some(expected.host)
        })
    })
}

pub(super) fn event_has_remem_subcommand_hook(
    doc: &Value,
    event: &str,
    executable: &Path,
    subcommand: &str,
    host: &str,
) -> bool {
    hook_commands_for_event(doc, event).any(|command| {
        parse_remem_invocation(command).is_some_and(|invocation| {
            Path::new(&invocation.executable) == executable
                && invocation.subcommand.as_deref() == Some(subcommand)
                && invocation.host.as_deref() == Some(host)
        })
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
    })
}

fn find_host_arg(tokens: &[String]) -> Option<String> {
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        if token == "--host" {
            return iter.next().cloned();
        }
        if let Some(host) = token.strip_prefix("--host=") {
            return Some(host.to_string());
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
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
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
            "REMEM_CONTEXT_HOST=codex-cli '/opt/remem bin/remem' context --host codex-cli",
        )
        .expect("remem invocation should parse");

        assert_eq!(
            invocation,
            RememInvocation {
                executable: "/opt/remem bin/remem".to_string(),
                subcommand: Some("context".to_string()),
                host: Some("codex-cli".to_string()),
            }
        );
    }

    #[test]
    fn parse_remem_invocation_handles_single_quote_escaping() {
        let invocation =
            parse_remem_invocation("'/tmp/remem'\\''bin/remem' observe --host claude-code")
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
}
