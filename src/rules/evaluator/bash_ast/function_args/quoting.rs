pub(super) fn quote_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| quote_argument(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn split_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| split_argument(argument))
        .filter(|argument| !argument.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn split_argument(value: &str) -> String {
    value
        .split([' ', '\t', '\n'])
        .filter(|word| !word.is_empty())
        .map(quote_argument)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn expand_at_in_double_quotes(arguments: &[String]) -> String {
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

pub(super) fn escape_double_quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

pub(super) fn quote_argument(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
