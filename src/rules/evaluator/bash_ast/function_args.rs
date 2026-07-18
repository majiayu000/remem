use brush_parser::ast::FunctionBody;

const MAX_DEFAULT_EXPANSION_DEPTH: usize = 8;

pub(super) fn expand_function_body(body: &FunctionBody, arguments: &[String]) -> String {
    let source = body.to_string();
    expand_positional_source(&source, None, arguments)
}

pub(super) fn expand_shell_command(
    source: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> String {
    expand_positional_source(source, zero_argument, arguments)
}

fn expand_positional_source(
    source: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> String {
    let mut expanded = String::with_capacity(source.len());
    let mut index = 0;
    let mut single_quoted = false;
    let mut double_quoted = false;
    while index < source.len() {
        let rest = &source[index..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if ch == '\'' && !double_quoted {
            single_quoted = !single_quoted;
            expanded.push(ch);
            index += 1;
            continue;
        }
        if ch == '"' && !single_quoted {
            if !double_quoted {
                if let Some((consumed, replacement)) =
                    quoted_collection_replacement(rest, arguments)
                {
                    expanded.push_str(&replacement);
                    index += consumed;
                    continue;
                }
            }
            double_quoted = !double_quoted;
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
        if ch == '$' && !single_quoted {
            if let Some((consumed, replacement)) =
                positional_replacement(rest, zero_argument, arguments, double_quoted, 0)
            {
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

fn quoted_collection_replacement(rest: &str, arguments: &[String]) -> Option<(usize, String)> {
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
    None
}

fn positional_replacement(
    rest: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    double_quoted: bool,
    depth: usize,
) -> Option<(usize, String)> {
    for pattern in ["${@}", "$@", "${*}", "$*"] {
        if rest.starts_with(pattern) {
            let replacement = if double_quoted {
                if pattern.contains('@') {
                    expand_at_in_double_quotes(arguments)
                } else {
                    escape_double_quoted(&arguments.join(" "))
                }
            } else {
                split_arguments(arguments)
            };
            return Some((pattern.len(), replacement));
        }
    }

    let (consumed, expression) = parameter_expression(rest)?;
    let value = resolve_parameter_expression(expression, zero_argument, arguments, depth)?;
    let replacement = if double_quoted {
        escape_double_quoted(&value)
    } else {
        split_argument(&value)
    };
    Some((consumed, replacement))
}

fn parameter_expression(rest: &str) -> Option<(usize, &str)> {
    let parameter = rest.strip_prefix('$')?;
    if parameter.starts_with('{') {
        let end = matching_parameter_brace(rest)?;
        return Some((end + 1, &rest[2..end]));
    }
    let first = parameter.chars().next()?;
    first
        .is_ascii_digit()
        .then_some((1 + first.len_utf8(), &parameter[..first.len_utf8()]))
}

fn matching_parameter_brace(source: &str) -> Option<usize> {
    let mut index = 2;
    let mut depth = 1;
    while index < source.len() {
        let rest = &source[index..];
        if rest.starts_with("${") {
            depth += 1;
            index += 2;
            continue;
        }
        let ch = rest.chars().next()?;
        if ch == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += ch.len_utf8();
    }
    None
}

fn resolve_parameter_expression(
    expression: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<String> {
    if depth > MAX_DEFAULT_EXPANSION_DEPTH {
        return None;
    }
    let digit_end = expression
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(expression.len());
    let digits = &expression[..digit_end];
    let position = digits.parse::<usize>().ok()?;
    if position == 0 && zero_argument.is_none() {
        return None;
    }
    let argument = if position == 0 {
        zero_argument
    } else {
        arguments.get(position - 1).map(String::as_str)
    };
    let suffix = &expression[digit_end..];
    let selected = if let Some(fallback) = suffix.strip_prefix(":-") {
        if argument.is_none_or(str::is_empty) {
            return expand_default_value(fallback, zero_argument, arguments, depth + 1);
        }
        argument
    } else if let Some(fallback) = suffix.strip_prefix('-') {
        if argument.is_none() {
            return expand_default_value(fallback, zero_argument, arguments, depth + 1);
        }
        argument
    } else if suffix.is_empty() {
        argument.or(Some(""))
    } else {
        return None;
    };
    Some(selected.unwrap_or("").to_string())
}

fn expand_default_value(
    source: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<String> {
    if depth > MAX_DEFAULT_EXPANSION_DEPTH {
        return None;
    }
    let mut expanded = String::with_capacity(source.len());
    let mut index = 0;
    let mut single_quoted = false;
    let mut double_quoted = false;
    while index < source.len() {
        let rest = &source[index..];
        let ch = rest.chars().next()?;
        if ch == '\'' && !double_quoted {
            single_quoted = !single_quoted;
            index += 1;
            continue;
        }
        if ch == '"' && !single_quoted {
            double_quoted = !double_quoted;
            index += 1;
            continue;
        }
        if ch == '\\' && !single_quoted {
            let next = source[index + 1..].chars().next()?;
            expanded.push(next);
            index += 1 + next.len_utf8();
            continue;
        }
        if ch == '$' && !single_quoted {
            if let Some(pattern) = ["${@}", "$@", "${*}", "$*"]
                .into_iter()
                .find(|pattern| rest.starts_with(pattern))
            {
                expanded.push_str(&arguments.join(" "));
                index += pattern.len();
                continue;
            }
            let (consumed, expression) = parameter_expression(rest)?;
            expanded.push_str(&resolve_parameter_expression(
                expression,
                zero_argument,
                arguments,
                depth,
            )?);
            index += consumed;
            continue;
        }
        if ch == '`' && !single_quoted {
            return None;
        }
        expanded.push(ch);
        index += ch.len_utf8();
    }
    (!single_quoted && !double_quoted).then_some(expanded)
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

fn expand_at_in_double_quotes(arguments: &[String]) -> String {
    let Some((first, remaining)) = arguments.split_first() else {
        return String::new();
    };
    let Some((last, middle)) = remaining.split_last() else {
        return escape_double_quoted(first);
    };

    let mut expanded = escape_double_quoted(first);
    expanded.push_str("\" ");
    for argument in middle {
        expanded.push_str(&quote_argument(argument));
        expanded.push(' ');
    }
    expanded.push_str(&quote_argument(last));
    expanded.push('"');
    expanded
}

fn escape_double_quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
