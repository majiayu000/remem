//! Static detection of execution contexts inside one command segment:
//! `shell -c` payloads, `eval` argument joins, `env -S` argv splitting, and
//! shells that read their script from stdin. Command position always comes
//! from the shared `unwrap` normalization layer.

use std::collections::HashMap;

use super::unwrap;
use super::DYNAMIC_SHELL_WORD;

pub(super) enum ExitTrapChange<'a> {
    Set(&'a str),
    Reset,
}

pub(super) fn direct_command_name(tokens: &[String]) -> Option<&str> {
    unwrap::direct_command_index(tokens).map(|index| unwrap::semantic_token(&tokens[index]))
}

pub(super) fn static_eval_payload(tokens: &[String]) -> Option<String> {
    let index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "eval" {
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

pub(super) fn static_exit_trap_change(tokens: &[String]) -> Option<ExitTrapChange<'_>> {
    let mut index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "trap" {
        return None;
    }
    index += 1;
    if matches!(tokens.get(index).map(String::as_str), Some("-p" | "-l")) {
        return None;
    }
    if tokens.get(index).is_some_and(|token| token == "--") {
        index += 1;
    }
    let payload = tokens.get(index)?;
    if payload == DYNAMIC_SHELL_WORD {
        return None;
    }
    let handles_exit = tokens[index + 1..]
        .iter()
        .any(|signal| signal == "0" || signal.eq_ignore_ascii_case("EXIT"));
    if !handles_exit {
        return None;
    }
    Some(if matches!(payload.as_str(), "" | "-") {
        ExitTrapChange::Reset
    } else {
        ExitTrapChange::Set(payload)
    })
}

pub(super) fn static_shopt_expand_aliases(tokens: &[String]) -> Option<bool> {
    static_shopt_state(tokens, "expand_aliases")
}

pub(super) fn static_shopt_lastpipe(tokens: &[String]) -> Option<bool> {
    static_shopt_state(tokens, "lastpipe")
}

pub(super) fn static_shopt_nocasematch(tokens: &[String]) -> Option<bool> {
    static_shopt_state(tokens, "nocasematch")
}

pub(super) fn static_monitor_mode(tokens: &[String]) -> Option<bool> {
    let mut index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "set" {
        return None;
    }
    index += 1;
    match tokens.get(index)?.as_str() {
        "-m" => Some(true),
        "+m" => Some(false),
        "-o" if tokens.get(index + 1).is_some_and(|name| name == "monitor") => Some(true),
        "+o" if tokens.get(index + 1).is_some_and(|name| name == "monitor") => Some(false),
        _ => None,
    }
}

pub(super) fn static_shell_exits(tokens: &[String]) -> bool {
    static_builtin_command_index(tokens)
        .and_then(|index| tokens.get(index))
        .is_some_and(|command| unwrap::semantic_token(command) == "exit")
}

fn static_shopt_state(tokens: &[String], expected: &str) -> Option<bool> {
    let mut index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "shopt" {
        return None;
    }
    index += 1;
    let enabled = match tokens.get(index)?.as_str() {
        "-s" => true,
        "-u" => false,
        _ => return None,
    };
    tokens[index + 1..]
        .iter()
        .any(|name| name == expected)
        .then_some(enabled)
}

pub(super) fn static_alias_definitions(tokens: &[String]) -> Option<Vec<(&str, &str)>> {
    let index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "alias" {
        return None;
    }
    Some(
        tokens[index + 1..]
            .iter()
            .filter_map(|definition| definition.split_once('='))
            .filter(|(name, payload)| {
                !name.is_empty()
                    && *payload != DYNAMIC_SHELL_WORD
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
            })
            .collect(),
    )
}

pub(super) fn static_unalias_names(tokens: &[String]) -> Option<Vec<&str>> {
    let mut index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "unalias" {
        return None;
    }
    index += 1;
    if tokens.get(index).is_some_and(|token| token == "--") {
        index += 1;
    }
    if tokens.get(index).is_some_and(|token| token == "-a") {
        return Some(Vec::new());
    }
    Some(tokens[index..].iter().map(String::as_str).collect())
}

pub(super) fn static_export_function_change(tokens: &[String]) -> Option<(bool, Vec<&str>)> {
    let mut index = static_builtin_command_index(tokens)?;
    let command = unwrap::semantic_token(tokens.get(index)?);
    let is_declare = matches!(command, "declare" | "typeset");
    if !is_declare && command != "export" {
        return None;
    }
    index += 1;
    let mut function_mode = false;
    let mut exported = !is_declare;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            index += 1;
            break;
        }
        let Some(flags) = option.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            break;
        };
        if flags.chars().any(|flag| !matches!(flag, 'f' | 'n' | 'x')) {
            return None;
        }
        function_mode |= flags.contains('f');
        if is_declare {
            exported |= flags.contains('x');
        } else {
            exported &= !flags.contains('n');
        }
        index += 1;
    }
    (function_mode && (!is_declare || exported)).then(|| {
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
    loop {
        match unwrap::semantic_token(tokens.get(index)?) {
            "command" => index = unwrap::command_wrapper_target(tokens, index)?,
            "builtin" => index += 1,
            _ => break,
        }
    }
    tokens.get(index).map(|_| index)
}

pub(super) fn static_env_split_tokens(tokens: &[String]) -> Option<Vec<String>> {
    let prefix_end = unwrap::direct_command_index(tokens)?;
    let assignments = static_assignments(&tokens[..prefix_end]);
    let mut index = prefix_end;
    while unwrap::semantic_token(tokens.get(index)?) == "command" {
        index = unwrap::command_wrapper_target(tokens, index)?;
    }
    if unwrap::semantic_token(tokens.get(index)?) != "env" {
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
                return splice_env_split(tokens, index, index + 2, payload, &assignments);
            }
            value if value.starts_with("--split-string=") => {
                let payload = value.strip_prefix("--split-string=")?;
                return splice_env_split(tokens, index, index + 1, payload, &assignments);
            }
            value if value.starts_with("-S") && value.len() > 2 => {
                return splice_env_split(tokens, index, index + 1, &value[2..], &assignments);
            }
            "-u" | "--unset" | "-C" | "--chdir" | "--argv0" => {
                tokens.get(index + 1)?;
                index += 2;
            }
            value
                if value.starts_with("--unset=")
                    || value.starts_with("--chdir=")
                    || value.starts_with("--argv0=")
                    || value.starts_with("--default-signal=")
                    || value.starts_with("--ignore-signal=")
                    || value.starts_with("--block-signal=") =>
            {
                index += 1;
            }
            "--default-signal"
            | "--ignore-signal"
            | "--block-signal"
            | "--list-signal-handling" => index += 1,
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
    assignments: &HashMap<&str, &str>,
) -> Option<Vec<String>> {
    if payload == DYNAMIC_SHELL_WORD {
        return None;
    }
    let payload = expand_env_split_variables(payload, assignments)?;
    let split = split_env_string(&payload)?;
    let mut expanded = Vec::with_capacity(tokens.len() + split.len());
    expanded.extend_from_slice(&tokens[..option_index]);
    expanded.extend(split);
    expanded.extend_from_slice(&tokens[suffix_index..]);
    Some(expanded)
}

fn static_assignments(tokens: &[String]) -> HashMap<&str, &str> {
    tokens
        .iter()
        .filter_map(|token| token.split_once('='))
        .filter(|(_, value)| *value != DYNAMIC_SHELL_WORD)
        .collect()
}

fn expand_env_split_variables(payload: &str, assignments: &HashMap<&str, &str>) -> Option<String> {
    let mut expanded = String::with_capacity(payload.len());
    let mut chars = payload.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            expanded.push(ch);
            expanded.push(chars.next()?);
            continue;
        }
        if ch != '$' || chars.peek() != Some(&'{') {
            expanded.push(ch);
            continue;
        }
        chars.next();
        let mut name = String::new();
        while chars.peek().is_some_and(|next| *next != '}') {
            name.push(chars.next()?);
        }
        chars.next()?;
        let valid_name = name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && name
                .chars()
                .all(|part| part.is_ascii_alphanumeric() || part == '_');
        if !valid_name {
            return None;
        }
        expanded.push_str(assignments.get(name.as_str())?);
    }
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
    let mut index = static_builtin_command_index(tokens)?;
    if unwrap::semantic_token(tokens.get(index)?) != "unset" {
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

pub(super) struct StaticShellCommand {
    pub(super) payload: String,
    pub(super) zero_argument: Option<String>,
    pub(super) arguments: Vec<String>,
}

pub(super) fn static_shell_command_payload(tokens: &[String]) -> Option<StaticShellCommand> {
    let command_index = unwrap::effective_command_index(tokens)?;
    if !is_shell(unwrap::semantic_token(tokens.get(command_index)?)) {
        return None;
    }
    let ShellInput::Command(payload_index) = shell_input(tokens, command_index) else {
        return None;
    };
    let payload = tokens.get(payload_index)?;
    if payload == DYNAMIC_SHELL_WORD {
        return None;
    }
    Some(StaticShellCommand {
        payload: payload.clone(),
        zero_argument: tokens.get(payload_index + 1).cloned(),
        arguments: tokens.get(payload_index + 2..).unwrap_or_default().to_vec(),
    })
}

pub(super) fn static_shell_reads_stdin(tokens: &[String]) -> bool {
    let Some(command_index) = unwrap::effective_command_index(tokens) else {
        return false;
    };
    if !tokens
        .get(command_index)
        .is_some_and(|command| is_shell(unwrap::semantic_token(command)))
    {
        return false;
    }
    shell_input(tokens, command_index) == ShellInput::Stdin
}

pub(super) fn static_shell_is_bash(tokens: &[String]) -> bool {
    unwrap::effective_command_index(tokens)
        .and_then(|index| tokens.get(index))
        .is_some_and(|command| shell_name(unwrap::semantic_token(command)) == Some("bash"))
}

pub(super) fn static_source_stdin_arguments(tokens: &[String]) -> Option<&[String]> {
    let index = static_builtin_command_index(tokens)?;
    if !matches!(unwrap::semantic_token(&tokens[index]), "source" | ".")
        || !tokens.get(index + 1).is_some_and(|path| {
            matches!(
                path.as_str(),
                "/dev/stdin" | "/dev/fd/0" | "/proc/self/fd/0"
            )
        })
    {
        return None;
    }
    Some(tokens.get(index + 2..).unwrap_or_default())
}

pub(super) fn static_set_positional_arguments(tokens: &[String]) -> Option<&[String]> {
    let index = static_builtin_command_index(tokens)?;
    (unwrap::semantic_token(&tokens[index]) == "set"
        && tokens.get(index + 1).is_some_and(|value| value == "--"))
    .then(|| tokens.get(index + 2..).unwrap_or_default())
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
    let mut reads_stdin = false;
    while let Some(option) = tokens.get(index) {
        if option == "--" {
            return if noexec {
                ShellInput::NoExec
            } else if reads_stdin || tokens.get(index + 1).is_none() {
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
                if option.starts_with('-') && flags.contains('s') {
                    reads_stdin = true;
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
        return if reads_stdin && !noexec {
            ShellInput::Stdin
        } else {
            ShellInput::Other
        };
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
    let name = command.rsplit(['/', '\\']).next()?;
    let normalized = name.strip_suffix(".exe").unwrap_or(name);
    matches!(normalized, "bash" | "dash" | "ksh" | "sh" | "zsh").then_some(normalized)
}

fn shell_option_takes_argument(option: &str) -> bool {
    matches!(
        option,
        "-O" | "+O" | "-o" | "+o" | "--init-file" | "--rcfile"
    )
}
