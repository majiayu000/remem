//! Unified command-position unwrapping.
//!
//! Single normalization layer for resolving where the effective program sits
//! inside one simple-command token list: environment-assignment prefixes
//! (including Bash `+=` append assignments) and the documented `command`,
//! `env`, and `exec` wrappers. Both the AST collector's static-execution
//! helpers and the structural evaluator resolve command position through
//! this module so wrapper semantics cannot drift apart.

/// Index of the first token that is not an environment-assignment prefix.
pub(crate) fn direct_command_index(tokens: &[String]) -> Option<usize> {
    tokens.iter().position(|token| !is_env_assignment(token))
}

/// Index of the effective program after recursively peeling assignment
/// prefixes and `command` / `env` / `exec` wrappers.
pub(crate) fn effective_command_index(tokens: &[String]) -> Option<usize> {
    let mut index = direct_command_index(tokens)?;
    loop {
        index = match tokens.get(index)?.as_str() {
            "command" => command_wrapper_target(tokens, index)?,
            "env" => env_wrapper_target(tokens, index)?,
            "exec" => exec_wrapper_target(tokens, index)?,
            _ => break,
        };
    }
    Some(index)
}

pub(crate) fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let name = name.strip_suffix('+').unwrap_or(name);
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(crate) fn command_wrapper_target(tokens: &[String], command_index: usize) -> Option<usize> {
    let mut index = command_index + 1;
    while let Some(option) = tokens.get(index) {
        match option.as_str() {
            "--" => return tokens.get(index + 1).map(|_| index + 1),
            "-p" => index += 1,
            "-v" | "-V" => return None,
            value if value.starts_with('-') => {
                let flags = value.strip_prefix('-')?;
                if flags.is_empty()
                    || flags.chars().any(|flag| !matches!(flag, 'p' | 'v' | 'V'))
                    || flags.chars().any(|flag| matches!(flag, 'v' | 'V'))
                {
                    return None;
                }
                index += 1;
            }
            _ => return Some(index),
        }
    }
    None
}

pub(crate) fn env_wrapper_target(tokens: &[String], command_index: usize) -> Option<usize> {
    let mut index = command_index + 1;
    let mut options_terminated = false;
    let mut assignments_started = false;
    while let Some(token) = tokens.get(index) {
        if is_env_assignment(token) {
            assignments_started = true;
            index += 1;
            continue;
        }
        if options_terminated || assignments_started {
            return Some(index);
        }
        match token.as_str() {
            "--" => {
                options_terminated = true;
                index += 1;
            }
            "-"
            | "-i"
            | "--ignore-environment"
            | "-0"
            | "--null"
            | "-v"
            | "--debug"
            | "--default-signal"
            | "--ignore-signal"
            | "--block-signal"
            | "--list-signal-handling" => index += 1,
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
                    || value.starts_with("--block-signal=")
                    || value.starts_with("-C") && value.len() > 2 =>
            {
                index += 1;
            }
            value if value.starts_with('-') => return None,
            _ => return Some(index),
        }
    }
    None
}

pub(crate) fn exec_wrapper_target(tokens: &[String], command_index: usize) -> Option<usize> {
    let mut index = command_index + 1;
    while let Some(option) = tokens.get(index) {
        match option.as_str() {
            "--" => return tokens.get(index + 1).map(|_| index + 1),
            "-a" => {
                tokens.get(index + 1)?;
                index += 2;
            }
            value if value.starts_with('-') && value.len() > 1 => {
                let flags = value.strip_prefix('-')?;
                if flags.starts_with('-') {
                    return None;
                }
                let flag_count = flags.chars().count();
                let mut consumes_next = false;
                for (position, flag) in flags.chars().enumerate() {
                    match flag {
                        'c' | 'l' => {}
                        'a' if position + 1 == flag_count => consumes_next = true,
                        _ => return None,
                    }
                }
                if consumes_next {
                    tokens.get(index + 1)?;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            _ => return Some(index),
        }
    }
    None
}
