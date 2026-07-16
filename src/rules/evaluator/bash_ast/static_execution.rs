//! Static detection of execution contexts inside one command segment:
//! `shell -c` payloads, `eval` argument joins, `env -S` argv splitting, and
//! shells that read their script from stdin. Command position always comes
//! from the shared `unwrap` normalization layer.

use super::unwrap;
use super::DYNAMIC_SHELL_WORD;

pub(super) fn direct_command_name(tokens: &[String]) -> Option<&str> {
    unwrap::direct_command_index(tokens).map(|index| tokens[index].as_str())
}

pub(super) fn static_eval_payload(tokens: &[String]) -> Option<String> {
    let index = static_builtin_command_index(tokens)?;
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

pub(super) fn static_exit_trap_payload(tokens: &[String]) -> Option<&str> {
    let mut index = static_builtin_command_index(tokens)?;
    if tokens.get(index)? != "trap" {
        return None;
    }
    index += 1;
    if tokens.get(index).is_some_and(|token| token == "--") {
        index += 1;
    }
    let payload = tokens.get(index)?;
    if matches!(payload.as_str(), "" | "-" | DYNAMIC_SHELL_WORD) {
        return None;
    }
    tokens[index + 1..]
        .iter()
        .any(|signal| signal == "0" || signal.eq_ignore_ascii_case("EXIT"))
        .then_some(payload.as_str())
}

pub(super) fn static_export_function_change(tokens: &[String]) -> Option<(bool, Vec<&str>)> {
    let mut index = static_builtin_command_index(tokens)?;
    if tokens.get(index)? != "export" {
        return None;
    }
    index += 1;
    let mut function_mode = false;
    let mut exported = true;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            index += 1;
            break;
        }
        let Some(flags) = option.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            break;
        };
        if flags.chars().any(|flag| !matches!(flag, 'f' | 'n')) {
            return None;
        }
        function_mode |= flags.contains('f');
        exported &= !flags.contains('n');
        index += 1;
    }
    function_mode.then(|| {
        (
            exported,
            tokens[index..]
                .iter()
                .filter(|name| name.as_str() != DYNAMIC_SHELL_WORD)
                .map(String::as_str)
                .collect(),
        )
    })
}

fn static_builtin_command_index(tokens: &[String]) -> Option<usize> {
    let mut index = unwrap::direct_command_index(tokens)?;
    while tokens.get(index)? == "command" {
        index = unwrap::command_wrapper_target(tokens, index)?;
    }
    if tokens.get(index)? == "builtin" {
        index += 1;
    }
    tokens.get(index).map(|_| index)
}

pub(super) fn static_env_split_tokens(tokens: &[String]) -> Option<Vec<String>> {
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
            return None;
        }
        match option.as_str() {
            "-S" | "--split-string" => {
                let payload = tokens.get(index + 1)?;
                return splice_env_split(tokens, index, index + 2, payload);
            }
            value if value.starts_with("--split-string=") => {
                let payload = value.strip_prefix("--split-string=")?;
                return splice_env_split(tokens, index, index + 1, payload);
            }
            value if value.starts_with("-S") && value.len() > 2 => {
                return splice_env_split(tokens, index, index + 1, &value[2..]);
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

fn splice_env_split(
    tokens: &[String],
    option_index: usize,
    suffix_index: usize,
    payload: &str,
) -> Option<Vec<String>> {
    if payload == DYNAMIC_SHELL_WORD {
        return None;
    }
    let split = split_env_string(payload)?;
    let mut expanded = Vec::with_capacity(tokens.len() + split.len());
    expanded.extend_from_slice(&tokens[..option_index]);
    expanded.extend(split);
    expanded.extend_from_slice(&tokens[suffix_index..]);
    Some(expanded)
}

fn split_env_string(payload: &str) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = payload.chars().peekable();
    let mut quote = Quote::None;
    let mut in_token = false;
    while let Some(ch) = chars.next() {
        match quote {
            Quote::None => match ch {
                ' ' | '\t' => finish_env_token(&mut tokens, &mut current, &mut in_token),
                '\'' => {
                    quote = Quote::Single;
                    in_token = true;
                }
                '"' => {
                    quote = Quote::Double;
                    in_token = true;
                }
                '#' if !in_token => break,
                '$' if chars.peek() == Some(&'{') => return None,
                '\\' => match chars.next()? {
                    'c' => {
                        break;
                    }
                    '_' => finish_env_token(&mut tokens, &mut current, &mut in_token),
                    escaped => {
                        current.push(env_escape(escaped)?);
                        in_token = true;
                    }
                },
                _ => {
                    current.push(ch);
                    in_token = true;
                }
            },
            Quote::Single => match ch {
                '\'' => quote = Quote::None,
                '\\' => {
                    let escaped = chars.next()?;
                    if matches!(escaped, '\'' | '\\') {
                        current.push(escaped);
                    } else {
                        current.push('\\');
                        current.push(escaped);
                    }
                }
                _ => current.push(ch),
            },
            Quote::Double => match ch {
                '"' => quote = Quote::None,
                '$' if chars.peek() == Some(&'{') => return None,
                '\\' => match chars.next()? {
                    'c' => return None,
                    '_' => current.push(' '),
                    escaped => current.push(env_escape(escaped)?),
                },
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

fn finish_env_token(tokens: &mut Vec<String>, current: &mut String, in_token: &mut bool) {
    if *in_token {
        tokens.push(std::mem::take(current));
        *in_token = false;
    }
}

fn env_escape(escaped: char) -> Option<char> {
    match escaped {
        'f' => Some('\u{000c}'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'v' => Some('\u{000b}'),
        '#' => Some('#'),
        '$' => Some('$'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '\\' => Some('\\'),
        _ => None,
    }
}

pub(super) fn static_unset_function_names(tokens: &[String]) -> Option<Vec<&str>> {
    let mut index = unwrap::direct_command_index(tokens)?;
    while tokens.get(index)? == "command" {
        index = unwrap::command_wrapper_target(tokens, index)?;
    }
    if tokens.get(index)? != "unset" {
        return None;
    }
    index += 1;
    let mut function_mode = false;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            index += 1;
            break;
        }
        let Some(flags) = option.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            break;
        };
        if flags.chars().any(|flag| flag != 'f') {
            return None;
        }
        function_mode = true;
        index += 1;
    }
    function_mode.then(|| {
        tokens[index..]
            .iter()
            .filter(|name| name.as_str() != DYNAMIC_SHELL_WORD)
            .map(String::as_str)
            .collect()
    })
}

pub(super) fn static_shell_command_payload(tokens: &[String]) -> Option<&str> {
    let command_index = unwrap::effective_command_index(tokens)?;
    if !is_shell(tokens.get(command_index)?) {
        return None;
    }
    let ShellInput::Command(payload_index) = shell_input(tokens, command_index) else {
        return None;
    };
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
    shell_input(tokens, command_index) == ShellInput::Stdin
}

pub(super) fn static_shell_is_bash(tokens: &[String]) -> bool {
    unwrap::effective_command_index(tokens)
        .and_then(|index| tokens.get(index))
        .is_some_and(|command| shell_name(command) == Some("bash"))
}

pub(super) fn static_source_reads_stdin(tokens: &[String]) -> bool {
    let Some(index) = static_builtin_command_index(tokens) else {
        return false;
    };
    matches!(tokens[index].as_str(), "source" | ".")
        && tokens
            .get(index + 1)
            .is_some_and(|path| path == "/dev/stdin")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShellInput {
    Command(usize),
    Stdin,
    Other,
    NoExec,
}

fn shell_input(tokens: &[String], command_index: usize) -> ShellInput {
    let mut index = command_index + 1;
    let mut noexec = false;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            return if noexec {
                ShellInput::NoExec
            } else if tokens.get(index + 1).is_none() {
                ShellInput::Stdin
            } else {
                ShellInput::Other
            };
        }
        if option == "-" {
            return if noexec {
                ShellInput::NoExec
            } else {
                ShellInput::Stdin
            };
        }
        if shell_option_takes_argument(option) {
            let Some(value) = tokens.get(index + 1) else {
                return ShellInput::Other;
            };
            if matches!(option.as_str(), "-o" | "+o") && value == "noexec" {
                noexec = option.starts_with('-');
            }
            index += 2;
            continue;
        }
        if option.starts_with('-') || option.starts_with('+') {
            if !option.starts_with("--") {
                let flags = &option[1..];
                if flags.contains('n') {
                    noexec = option.starts_with('-');
                }
                if option.starts_with('-') && flags.contains('c') {
                    return if noexec {
                        ShellInput::NoExec
                    } else if tokens.get(index + 1).is_some() {
                        ShellInput::Command(index + 1)
                    } else {
                        ShellInput::Other
                    };
                }
            }
            index += 1;
            continue;
        }
        return ShellInput::Other;
    }
    if noexec {
        ShellInput::NoExec
    } else {
        ShellInput::Stdin
    }
}

fn is_shell(command: &str) -> bool {
    shell_name(command).is_some()
}

fn shell_name(command: &str) -> Option<&str> {
    std::path::Path::new(command)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|name| matches!(*name, "bash" | "dash" | "ksh" | "sh" | "zsh"))
}

fn shell_option_takes_argument(option: &str) -> bool {
    matches!(
        option,
        "-O" | "+O" | "-o" | "+o" | "--init-file" | "--rcfile"
    )
}
