//! Static detection of execution contexts inside one command segment:
//! `shell -c` payloads, `eval` argument joins, `env -S` split strings, and
//! shells that read their script from stdin. Command position always comes
//! from the shared `unwrap` normalization layer.

use super::unwrap;
use super::DYNAMIC_SHELL_WORD;

pub(super) fn direct_command_name(tokens: &[String]) -> Option<&str> {
    unwrap::direct_command_index(tokens).map(|index| tokens[index].as_str())
}

pub(super) fn static_eval_payload(tokens: &[String]) -> Option<String> {
    let mut index = unwrap::direct_command_index(tokens)?;
    while tokens.get(index)? == "command" {
        index = unwrap::command_wrapper_target(tokens, index)?;
    }
    if tokens.get(index)? != "eval" {
        return None;
    }
    let mut arguments = &tokens[index + 1..];
    if arguments.first().is_some_and(|argument| argument == "--") {
        arguments = &arguments[1..];
    }
    (!arguments.is_empty()
        && arguments
            .iter()
            .all(|argument| argument != DYNAMIC_SHELL_WORD))
    .then(|| arguments.join(" "))
}

pub(super) fn static_env_split_payload(tokens: &[String]) -> Option<String> {
    let mut index = unwrap::direct_command_index(tokens)?;
    while tokens.get(index)? == "command" {
        index = unwrap::command_wrapper_target(tokens, index)?;
    }
    if tokens.get(index)? != "env" {
        return None;
    }
    index += 1;
    while let Some(option) = tokens.get(index) {
        if unwrap::is_env_assignment(option) {
            index += 1;
            continue;
        }
        match option.as_str() {
            "-S" | "--split-string" => {
                let payload = tokens.get(index + 1)?;
                return (payload != DYNAMIC_SHELL_WORD).then(|| payload.clone());
            }
            value if value.starts_with("--split-string=") => {
                return value
                    .strip_prefix("--split-string=")
                    .filter(|payload| *payload != DYNAMIC_SHELL_WORD)
                    .map(str::to_string);
            }
            value if value.starts_with("-S") && value.len() > 2 => {
                return Some(value[2..].to_string());
            }
            "-u" | "--unset" | "-C" | "--chdir" | "--argv0" => {
                tokens.get(index + 1)?;
                index += 2;
            }
            value
                if value.starts_with("--unset=")
                    || value.starts_with("--chdir=")
                    || value.starts_with("--argv0=") =>
            {
                index += 1;
            }
            "--" => return None,
            value if value.starts_with('-') => index += 1,
            _ => return None,
        }
    }
    None
}

pub(super) fn static_shell_command_payload(tokens: &[String]) -> Option<&str> {
    let command_index = unwrap::effective_command_index(tokens)?;
    if !is_shell(tokens.get(command_index)?) {
        return None;
    }
    let payload_index = shell_command_payload_index(tokens, command_index)?;
    let payload = tokens.get(payload_index)?;
    (payload != DYNAMIC_SHELL_WORD).then_some(payload.as_str())
}

pub(super) fn static_shell_reads_stdin(tokens: &[String]) -> bool {
    let Some(command_index) = unwrap::effective_command_index(tokens) else {
        return false;
    };
    if !tokens
        .get(command_index)
        .is_some_and(|command| is_shell(command))
    {
        return false;
    }
    let mut index = command_index + 1;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            return tokens.get(index + 1).is_none();
        }
        if option == "-" {
            return true;
        }
        if shell_option_takes_argument(option) {
            index += 2;
            continue;
        }
        if shell_option_carries_command(option) {
            return false;
        }
        if option.starts_with('-') || option.starts_with('+') {
            index += 1;
            continue;
        }
        return false;
    }
    true
}

fn shell_command_payload_index(tokens: &[String], command_index: usize) -> Option<usize> {
    let mut index = command_index + 1;
    while let Some(option) = tokens.get(index) {
        if option == "--" || option == "-" {
            return None;
        }
        if shell_option_takes_argument(option) {
            tokens.get(index + 1)?;
            index += 2;
            continue;
        }
        if shell_option_carries_command(option) {
            return tokens.get(index + 1).map(|_| index + 1);
        }
        if option.starts_with('-') || option.starts_with('+') {
            index += 1;
            continue;
        }
        return None;
    }
    None
}

fn is_shell(command: &str) -> bool {
    std::path::Path::new(command)
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| matches!(name, "bash" | "dash" | "ksh" | "sh" | "zsh"))
}

fn shell_option_takes_argument(option: &str) -> bool {
    matches!(
        option,
        "-O" | "+O" | "-o" | "+o" | "--init-file" | "--rcfile"
    )
}

fn shell_option_carries_command(option: &str) -> bool {
    option == "-c"
        || option
            .strip_prefix('-')
            .is_some_and(|flags| !flags.starts_with('-') && flags.contains('c'))
}
