use brush_parser::ast::FunctionBody;
use brush_parser::word::{WordPiece, WordPieceWithSource};
use brush_parser::ParserOptions;

use super::positional_slice::{positional_collection_slice, positional_substring};

const MAX_DEFAULT_EXPANSION_DEPTH: usize = 8;

pub(super) fn expand_function_body(body: &FunctionBody, arguments: &[String]) -> String {
    let source = body.to_string();
    expand_positional_source(&source, None, arguments)
}

pub(super) fn expand_shell_command(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> Result<String, String> {
    let pieces = brush_parser::word::parse(source, options)
        .map_err(|error| format!("Bash positional word parse error: {error}"))?;
    Ok(expand_word_range(
        source,
        &pieces,
        0,
        source.len(),
        false,
        zero_argument,
        arguments,
        true,
    ))
}

pub(super) fn expand_shell_here_string(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> Result<String, String> {
    let pieces = brush_parser::word::parse(source, options)
        .map_err(|error| format!("Bash positional here-string parse error: {error}"))?;
    Ok(expand_word_range(
        source,
        &pieces,
        0,
        source.len(),
        false,
        zero_argument,
        arguments,
        false,
    ))
}

pub(super) fn has_shell_positional_reference(source: &str) -> bool {
    let bytes = source.as_bytes();
    (0..bytes.len()).any(|index| {
        if bytes[index] != b'$' {
            return false;
        }
        let next = bytes.get(index + 1).copied();
        next.is_some_and(|value| value.is_ascii_digit() || matches!(value, b'@' | b'*'))
            || next == Some(b'{')
                && bytes
                    .get(index + 2)
                    .is_some_and(|value| value.is_ascii_digit() || matches!(value, b'@' | b'*'))
    })
}

pub(super) fn expand_shell_heredoc(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> Result<String, String> {
    let pieces = brush_parser::word::parse_heredoc(source, options)
        .map_err(|error| format!("Bash positional heredoc parse error: {error}"))?;
    Ok(expand_heredoc_range(
        source,
        &pieces,
        zero_argument,
        arguments,
    ))
}

pub(super) fn expand_shell_arithmetic(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> Result<String, String> {
    let pieces = brush_parser::word::parse(source, options)
        .map_err(|error| format!("Bash positional arithmetic parse error: {error}"))?;
    Ok(expand_heredoc_range(
        source,
        &pieces,
        zero_argument,
        arguments,
    ))
}

pub(super) fn bare_shell_positional_fields(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
) -> Result<Option<Vec<String>>, String> {
    if matches!(source, r#""$@""# | r#""${@}""#) {
        return Ok(Some(arguments.to_vec()));
    }
    if let Some(inner) = source
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        if matches!(inner, "$*" | "${*}") {
            return Ok(Some(vec![arguments.join(" ")]));
        }
        if let Some((consumed, expression)) = parameter_expression(inner) {
            if consumed == inner.len() {
                if let Some(selected) =
                    positional_collection_slice(expression, zero_argument, arguments)
                {
                    return Ok(Some(selected));
                }
                if expression.starts_with(|ch: char| ch.is_ascii_digit()) {
                    let value =
                        resolve_parameter_expression(expression, zero_argument, arguments, 0);
                    return Ok(value.map(|value| vec![value]));
                }
            }
        }
    }
    let pieces = brush_parser::word::parse(source, options)
        .map_err(|error| format!("Bash positional field parse error: {error}"))?;
    let [piece] = pieces.as_slice() else {
        return Ok(None);
    };
    if piece.start_index != 0
        || piece.end_index != source.len()
        || !matches!(piece.piece, WordPiece::ParameterExpansion(_))
    {
        return Ok(None);
    }
    let raw = &source[piece.start_index..piece.end_index];
    let Some(fields) = positional_source_fields(raw, zero_argument, arguments, 0) else {
        return Ok(None);
    };
    Ok(Some(fields))
}

pub(super) fn bare_shell_positional_variant_fields(
    source: &str,
    options: &ParserOptions,
    zero_argument: Option<&str>,
    arguments: &[String],
    possible_arguments: &[Vec<String>],
) -> Result<Option<Vec<String>>, String> {
    let mut combined = Vec::new();
    for arguments in std::iter::once(arguments).chain(possible_arguments.iter().map(Vec::as_slice))
    {
        let Some(fields) = bare_shell_positional_fields(source, options, zero_argument, arguments)?
        else {
            return Ok(None);
        };
        combined.extend(fields);
    }
    Ok(Some(combined))
}

fn positional_source_fields(
    source: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<Vec<String>> {
    if matches!(source, "${@}" | "$@" | "${*}" | "$*") {
        return Some(
            arguments
                .iter()
                .flat_map(|value| split_fields(value))
                .collect(),
        );
    }
    let (consumed, expression) = parameter_expression(source)?;
    (consumed == source.len())
        .then(|| resolve_parameter_expression_fields(expression, zero_argument, arguments, depth))?
}

fn resolve_parameter_expression_fields(
    expression: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<Vec<String>> {
    if depth > MAX_DEFAULT_EXPANSION_DEPTH {
        return None;
    }
    if let Some(arguments) = positional_collection_slice(expression, zero_argument, arguments) {
        return Some(
            arguments
                .iter()
                .flat_map(|value| split_fields(value))
                .collect(),
        );
    }
    let digit_end = expression
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(expression.len());
    let position = expression[..digit_end].parse::<usize>().ok()?;
    if position == 0 && zero_argument.is_none() {
        return None;
    }
    let argument = if position == 0 {
        zero_argument
    } else {
        arguments.get(position - 1).map(String::as_str)
    };
    let suffix = &expression[digit_end..];
    if let Some(slice) = suffix
        .strip_prefix(':')
        .filter(|slice| slice.starts_with(|ch: char| ch.is_ascii_digit()))
    {
        return Some(split_fields(&positional_substring(
            argument.unwrap_or_default(),
            slice,
        )?));
    }
    if let Some(fallback) = suffix.strip_prefix(":-") {
        return if argument.is_none_or(str::is_empty) {
            expand_default_fields(fallback, zero_argument, arguments, depth + 1)
        } else {
            Some(split_fields(argument.unwrap_or_default()))
        };
    }
    if let Some(fallback) = suffix.strip_prefix('-') {
        return if argument.is_none() {
            expand_default_fields(fallback, zero_argument, arguments, depth + 1)
        } else {
            Some(split_fields(argument.unwrap_or_default()))
        };
    }
    if let Some(alternative) = suffix.strip_prefix(":+") {
        return if argument.is_some_and(|value| !value.is_empty()) {
            expand_default_fields(alternative, zero_argument, arguments, depth + 1)
        } else {
            Some(Vec::new())
        };
    }
    if let Some(alternative) = suffix.strip_prefix('+') {
        return if argument.is_some() {
            expand_default_fields(alternative, zero_argument, arguments, depth + 1)
        } else {
            Some(Vec::new())
        };
    }
    suffix
        .is_empty()
        .then(|| split_fields(argument.unwrap_or_default()))
}

fn expand_default_fields(
    source: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<Vec<String>> {
    if depth > MAX_DEFAULT_EXPANSION_DEPTH {
        return None;
    }
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut field_started = false;
    let mut index = 0;
    let mut single_quoted = false;
    let mut double_quoted = false;
    while index < source.len() {
        let rest = &source[index..];
        let ch = rest.chars().next()?;
        if ch == '\'' && !double_quoted {
            single_quoted = !single_quoted;
            field_started = true;
            index += 1;
            continue;
        }
        if ch == '"' && !single_quoted {
            double_quoted = !double_quoted;
            field_started = true;
            index += 1;
            continue;
        }
        if ch == '\\' && !single_quoted {
            let next = source[index + 1..].chars().next()?;
            field.push(next);
            field_started = true;
            index += 1 + next.len_utf8();
            continue;
        }
        if ch == '$' && !single_quoted {
            let (consumed, expression) = parameter_expression(rest)?;
            let nested = resolve_parameter_expression_fields(
                expression,
                zero_argument,
                arguments,
                depth + 1,
            )?;
            if double_quoted {
                field.push_str(&nested.join(" "));
                field_started = true;
            } else {
                append_expanded_fields(&mut fields, &mut field, &mut field_started, nested);
            }
            index += consumed;
            continue;
        }
        if ch == '`' && !single_quoted {
            return None;
        }
        if ch.is_ascii_whitespace() && !single_quoted && !double_quoted {
            if field_started {
                fields.push(std::mem::take(&mut field));
                field_started = false;
            }
        } else {
            field.push(ch);
            field_started = true;
        }
        index += ch.len_utf8();
    }
    if single_quoted || double_quoted {
        return None;
    }
    if field_started {
        fields.push(field);
    }
    Some(fields)
}

fn append_expanded_fields(
    fields: &mut Vec<String>,
    field: &mut String,
    field_started: &mut bool,
    expanded: Vec<String>,
) {
    let Some((first, remaining)) = expanded.split_first() else {
        return;
    };
    field.push_str(first);
    *field_started = true;
    for value in remaining {
        fields.push(std::mem::take(field));
        field.push_str(value);
    }
}

fn split_fields(value: &str) -> Vec<String> {
    value
        .split([' ', '\t', '\n'])
        .filter(|field| !field.is_empty())
        .map(str::to_string)
        .collect()
}

fn expand_word_range(
    source: &str,
    pieces: &[WordPieceWithSource],
    range_start: usize,
    range_end: usize,
    double_quoted: bool,
    zero_argument: Option<&str>,
    arguments: &[String],
    split_unquoted: bool,
) -> String {
    let mut expanded = String::with_capacity(range_end.saturating_sub(range_start));
    let mut cursor = range_start;
    for piece in pieces {
        expanded.push_str(&source[cursor..piece.start_index]);
        match &piece.piece {
            WordPiece::DoubleQuotedSequence(inner)
            | WordPiece::GettextDoubleQuotedSequence(inner) => {
                expanded.push_str(&expand_word_range(
                    source,
                    inner,
                    piece.start_index,
                    piece.end_index,
                    true,
                    zero_argument,
                    arguments,
                    split_unquoted,
                ));
            }
            WordPiece::ParameterExpansion(_) => {
                let raw = &source[piece.start_index..piece.end_index];
                if let Some((consumed, replacement)) = positional_replacement(
                    raw,
                    zero_argument,
                    arguments,
                    double_quoted,
                    split_unquoted,
                    0,
                ) {
                    if consumed == raw.len() {
                        expanded.push_str(&replacement);
                    } else {
                        expanded.push_str(raw);
                    }
                } else {
                    expanded.push_str(raw);
                }
            }
            _ => expanded.push_str(&source[piece.start_index..piece.end_index]),
        }
        cursor = piece.end_index;
    }
    expanded.push_str(&source[cursor..range_end]);
    expanded
}

fn expand_heredoc_range(
    source: &str,
    pieces: &[WordPieceWithSource],
    zero_argument: Option<&str>,
    arguments: &[String],
) -> String {
    let mut expanded = String::with_capacity(source.len());
    let mut cursor = 0;
    for piece in pieces {
        expanded.push_str(&source[cursor..piece.start_index]);
        if matches!(piece.piece, WordPiece::ParameterExpansion(_)) {
            let raw = &source[piece.start_index..piece.end_index];
            if let Some(value) = positional_source_value(raw, zero_argument, arguments, 0) {
                expanded.push_str(&value);
            } else {
                expanded.push_str(raw);
            }
        } else {
            expanded.push_str(&source[piece.start_index..piece.end_index]);
        }
        cursor = piece.end_index;
    }
    expanded.push_str(&source[cursor..]);
    expanded
}

fn positional_source_value(
    source: &str,
    zero_argument: Option<&str>,
    arguments: &[String],
    depth: usize,
) -> Option<String> {
    if matches!(source, "${@}" | "$@" | "${*}" | "$*") {
        return Some(arguments.join(" "));
    }
    let (consumed, expression) = parameter_expression(source)?;
    (consumed == source.len())
        .then(|| resolve_parameter_expression(expression, zero_argument, arguments, depth))?
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
                positional_replacement(rest, zero_argument, arguments, double_quoted, true, 0)
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
    split_unquoted: bool,
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
            } else if !split_unquoted {
                quote_argument(&arguments.join(" "))
            } else {
                split_arguments(arguments)
            };
            return Some((pattern.len(), replacement));
        }
    }

    let (consumed, expression) = parameter_expression(rest)?;
    if let Some(arguments) = positional_collection_slice(expression, zero_argument, arguments) {
        let replacement = if double_quoted {
            expand_at_in_double_quotes(&arguments)
        } else if !split_unquoted {
            quote_argument(&arguments.join(" "))
        } else {
            split_arguments(&arguments)
        };
        return Some((consumed, replacement));
    }
    let value = resolve_parameter_expression(expression, zero_argument, arguments, depth)?;
    let replacement = if double_quoted {
        escape_double_quoted(&value)
    } else if !split_unquoted {
        quote_argument(&value)
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
    if let Some(arguments) = positional_collection_slice(expression, zero_argument, arguments) {
        return Some(arguments.join(" "));
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
    if let Some(slice) = suffix
        .strip_prefix(':')
        .filter(|slice| slice.starts_with(|ch: char| ch.is_ascii_digit()))
    {
        return positional_substring(argument.unwrap_or_default(), slice);
    }
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
    } else if let Some(alternative) = suffix.strip_prefix(":+") {
        if argument.is_some_and(|value| !value.is_empty()) {
            return expand_default_value(alternative, zero_argument, arguments, depth + 1);
        }
        Some("")
    } else if let Some(alternative) = suffix.strip_prefix('+') {
        if argument.is_some() {
            return expand_default_value(alternative, zero_argument, arguments, depth + 1);
        }
        Some("")
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
