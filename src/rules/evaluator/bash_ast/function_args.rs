use brush_parser::ast::FunctionBody;

pub(super) fn expand_function_body(body: &FunctionBody, arguments: &[String]) -> String {
    let source = body.to_string();
    let mut expanded = String::with_capacity(source.len());
    let mut index = 0;
    let mut single_quoted = false;
    while index < source.len() {
        let rest = &source[index..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '\'' {
            single_quoted = !single_quoted;
            expanded.push(ch);
            index += 1;
            continue;
        }
        if ch == '\\' {
            expanded.push(ch);
            index += 1;
            if let Some(next) = source[index..].chars().next() {
                expanded.push(next);
                index += next.len_utf8();
            }
            continue;
        }
        if !single_quoted {
            if let Some((consumed, replacement)) = positional_replacement(rest, arguments) {
                expanded.push_str(&replacement);
                index += consumed;
                continue;
            }
        }
        expanded.push(ch);
        index += ch.len_utf8();
    }
    expanded
}

fn positional_replacement(rest: &str, arguments: &[String]) -> Option<(usize, String)> {
    for pattern in ["\"${@}\"", "\"$@\""] {
        if rest.starts_with(pattern) {
            return Some((pattern.len(), quote_arguments(arguments)));
        }
    }
    for pattern in ["\"${*}\"", "\"$*\""] {
        if rest.starts_with(pattern) {
            return Some((pattern.len(), quote_argument(&arguments.join(" "))));
        }
    }
    for pattern in ["${@}", "$@", "${*}", "$*"] {
        if rest.starts_with(pattern) {
            return Some((pattern.len(), split_arguments(arguments)));
        }
    }

    let quoted = rest.starts_with('\"');
    let start = usize::from(quoted);
    let parameter = rest.get(start..)?.strip_prefix('$')?;
    let (expression, parameter_len) = if let Some(braced) = parameter.strip_prefix('{') {
        let end = braced.find('}')?;
        (&braced[..end], end + 2)
    } else {
        let first = parameter.chars().next()?;
        if !first.is_ascii_digit() {
            return None;
        }
        (&parameter[..first.len_utf8()], first.len_utf8())
    };
    let (digits, default) = parse_parameter_expression(expression)?;
    let position = digits.parse::<usize>().ok()?;
    if position == 0 {
        return None;
    }
    let mut consumed = start + 1 + parameter_len;
    if quoted {
        if rest.as_bytes().get(consumed) != Some(&b'\"') {
            return None;
        }
        consumed += 1;
    }
    let argument = arguments.get(position - 1).map(String::as_str);
    let value = match default {
        Some((true, fallback)) if argument.is_none_or(str::is_empty) => fallback,
        Some((false, fallback)) if argument.is_none() => fallback,
        _ => argument.unwrap_or(""),
    };
    let replacement = if quoted {
        quote_argument(value)
    } else {
        split_argument(value)
    };
    Some((consumed, replacement))
}

fn parse_parameter_expression(expression: &str) -> Option<(&str, Option<(bool, &str)>)> {
    let digit_end = expression
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(expression.len());
    let digits = &expression[..digit_end];
    if digits.is_empty() {
        return None;
    }
    let suffix = &expression[digit_end..];
    let default = if let Some(fallback) = suffix.strip_prefix(":-") {
        Some((true, fallback))
    } else if let Some(fallback) = suffix.strip_prefix('-') {
        Some((false, fallback))
    } else if suffix.is_empty() {
        None
    } else {
        return None;
    };
    Some((digits, default))
}

fn quote_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| quote_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| split_argument(argument))
        .filter(|argument| !argument.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn split_argument(value: &str) -> String {
    value
        .split([' ', '\t', '\n'])
        .filter(|word| !word.is_empty())
        .map(quote_argument)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
