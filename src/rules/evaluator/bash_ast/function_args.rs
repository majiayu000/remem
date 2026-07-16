use brush_parser::ast::FunctionBody;

pub(super) fn expand_function_body(body: &FunctionBody, arguments: &[String]) -> String {
    let source = body.to_string();
    let mut expanded = String::with_capacity(source.len());
    let mut index = 0;
    let mut single_quoted = false;
    while index < source.len() {
        let rest = &source[index..];
        let ch = rest.chars().next().expect("non-empty source tail");
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
            return Some((pattern.len(), quote_arguments(arguments)));
        }
    }

    let quoted = rest.starts_with('\"');
    let start = usize::from(quoted);
    let parameter = rest.get(start..)?.strip_prefix('$')?;
    let (digits, parameter_len) = if let Some(braced) = parameter.strip_prefix('{') {
        let end = braced.find('}')?;
        (&braced[..end], end + 2)
    } else {
        let end = parameter
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(parameter.len());
        (&parameter[..end], end)
    };
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
    let value = arguments.get(position - 1).map_or("", String::as_str);
    Some((consumed, quote_argument(value)))
}

fn quote_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| quote_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
