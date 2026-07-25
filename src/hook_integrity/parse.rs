use std::path::Path;

use serde_json::Value;

use super::RememInvocation;

pub(super) fn parse_remem_hook_value(hook: &Value) -> Option<RememInvocation> {
    let command = hook.get("command").and_then(|command| command.as_str())?;
    if let Some(args) = hook.get("args").and_then(|args| args.as_array()) {
        let mut tokens = vec![command.to_string()];
        for arg in args {
            tokens.push(arg.as_str()?.to_string());
        }
        return parse_remem_tokens(tokens);
    }
    parse_remem_invocation(command)
}

pub(super) fn parse_remem_invocation(command: &str) -> Option<RememInvocation> {
    parse_remem_tokens(shell_words(command)?)
}

fn parse_remem_tokens(tokens: Vec<String>) -> Option<RememInvocation> {
    let command_index = tokens.iter().position(|token| !is_env_prefix(token))?;
    if !is_remem_command_token(&tokens[command_index]) {
        return None;
    }
    let host = find_host_arg(&tokens[command_index + 1..]);

    Some(RememInvocation {
        executable: tokens[command_index].clone(),
        subcommand: tokens.get(command_index + 1).cloned(),
        nested_subcommand: tokens
            .get(command_index + 2)
            .filter(|token| !token.starts_with('-'))
            .cloned(),
        host,
        env_host: find_legacy_host_env(&tokens[..command_index]),
    })
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

fn is_env_prefix(token: &str) -> bool {
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
    fn parses_nested_rules_eval_without_treating_host_as_subcommand() {
        let invocation = parse_remem_invocation("/tmp/remem rules eval --host claude-code")
            .expect("rules eval invocation");
        assert_eq!(invocation.subcommand.as_deref(), Some("rules"));
        assert_eq!(invocation.nested_subcommand.as_deref(), Some("eval"));
        assert_eq!(invocation.resolved_host(), Some("claude-code"));

        let flat = parse_remem_invocation("/tmp/remem observe --host claude-code")
            .expect("observe invocation");
        assert_eq!(flat.nested_subcommand, None);
    }
}
