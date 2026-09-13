//! Token walks, shell-word parsing, and inline assignment redaction.

use super::keys::{
    is_sensitive_key, is_sensitive_option_key, key_owns_header_value, redact_token_with_options,
};
use super::redact_token;

pub(super) fn redact_tokens(
    line: &str,
    redact_sensitive_options: bool,
    preserve_filesystem_paths: bool,
) -> String {
    let mut previous_was_bearer = false;
    let mut previous_was_sensitive_option = false;
    let mut output = String::with_capacity(line.len());
    let mut rest = line;

    while !rest.is_empty() {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            // Preserve trailing whitespace exactly.
            output.push_str(rest);
            break;
        }
        let leading_ws_len = rest.len() - trimmed.len();
        output.push_str(&rest[..leading_ws_len]);

        let group_quotes = previous_was_bearer
            || previous_was_sensitive_option
            || (redact_sensitive_options && attached_sensitive_short_option_starts_quoted(trimmed));

        let Some((token, remaining)) = next_redaction_token(trimmed, group_quotes) else {
            // Preserve standalone shell operators (e.g. `&&`, `&`) instead of
            // truncating the rebuild when a pending sensitive argument is absent.
            if let Some((op, after)) = take_leading_shell_operator(trimmed) {
                output.push_str(op);
                previous_was_bearer = false;
                previous_was_sensitive_option = false;
                rest = after;
                continue;
            }
            break;
        };

        let redacted = if previous_was_bearer || previous_was_sensitive_option {
            "[REDACTED]".to_string()
        } else if redact_sensitive_options {
            if let Some(attached) = redact_attached_sensitive_short_option(&token) {
                attached
            } else {
                redact_token_with_options(&token, preserve_filesystem_paths)
            }
        } else {
            redact_token_with_options(&token, preserve_filesystem_paths)
        };
        previous_was_sensitive_option =
            redact_sensitive_options && token_expects_sensitive_argument(&token);
        previous_was_bearer = token
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
            .eq_ignore_ascii_case("bearer");
        output.push_str(&redacted);
        rest = remaining;
    }

    output
}

pub(super) fn tokens_contain_sensitive_match(
    line: &str,
    redact_sensitive_options: bool,
    detect_attached_short_options: bool,
) -> bool {
    let mut previous_was_bearer = false;
    let mut previous_was_sensitive_option = false;
    let mut rest = line;
    while !rest.is_empty() {
        let trimmed = rest.trim_start();
        if trimmed.is_empty() {
            break;
        }
        let group_quotes = previous_was_bearer
            || previous_was_sensitive_option
            || (redact_sensitive_options
                && detect_attached_short_options
                && attached_sensitive_short_option_starts_quoted(trimmed));
        let Some((token, remaining)) = next_redaction_token(trimmed, group_quotes) else {
            if let Some((_, after)) = take_leading_shell_operator(trimmed) {
                previous_was_bearer = false;
                previous_was_sensitive_option = false;
                rest = after;
                continue;
            }
            break;
        };
        if previous_was_bearer
            || previous_was_sensitive_option
            || (redact_sensitive_options
                && detect_attached_short_options
                && redact_attached_sensitive_short_option(&token).is_some())
            || redact_token(&token) != token
        {
            return true;
        }
        previous_was_sensitive_option =
            redact_sensitive_options && token_expects_sensitive_argument(&token);
        previous_was_bearer = token
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
            .eq_ignore_ascii_case("bearer");
        rest = remaining;
    }
    false
}

/// Take the next token for redaction.
///
/// Quote grouping is only enabled while consuming a known sensitive option or
/// Bearer argument, or an attached quoted short-option credential, so benign
/// phrases like `Review "phase 2 migration …"` are not glued into one
/// length-heuristic hit.
fn next_redaction_token(line: &str, group_quotes: bool) -> Option<(String, &str)> {
    if line.is_empty() {
        return None;
    }
    if group_quotes {
        take_shell_like_argument(line)
    } else {
        take_whitespace_token(line)
    }
}

fn take_whitespace_token(line: &str) -> Option<(String, &str)> {
    let end = line
        .char_indices()
        .find_map(|(idx, ch)| ch.is_whitespace().then_some(idx))
        .unwrap_or(line.len());
    Some((line[..end].to_string(), &line[end..]))
}

/// Consume one shell-like argument, keeping a quoted span intact so
/// `-u "alice:correct horse"` redacts as a single value.
///
/// Unquoted arguments also honor backslash-escaped whitespace so
/// `--token correct\ horse` is consumed as one credential.
/// Adjacent quote concatenation such as `"abc"tail` is one shell word.
pub(super) fn take_shell_like_argument(line: &str) -> Option<(String, &str)> {
    if line.is_empty() {
        return None;
    }
    let mut token = String::new();
    let end = extend_glued_shell_word(line, 0, &mut token);
    if token.is_empty() {
        return None;
    }
    Some((token, &line[end..]))
}

fn extend_glued_shell_word(line: &str, start: usize, token: &mut String) -> usize {
    let mut end = start;
    let mut escaped = false;
    while end < line.len() {
        let ch = line[end..].chars().next().expect("end in bounds");
        if escaped {
            token.push(ch);
            escaped = false;
            end += ch.len_utf8();
            continue;
        }
        if ch == '\\' {
            token.push(ch);
            escaped = true;
            end += ch.len_utf8();
            continue;
        }
        if is_shell_word_break(ch) {
            break;
        }
        if ch == '$' {
            token.push(ch);
            end += ch.len_utf8();
            // Command substitutions such as `$(printf %s secret)` are one shell
            // word; consume the balanced parentheses instead of stopping at the
            // first whitespace inside `$()`.
            if line[end..].starts_with('(') {
                end = consume_balanced_shell_group(line, end, '(', ')', token);
            } else if line[end..].starts_with('{') {
                end = consume_balanced_shell_group(line, end, '{', '}', token);
            }
            continue;
        }
        if ch == '`' {
            // Backtick command substitution is also one shell word.
            token.push(ch);
            end += ch.len_utf8();
            while end < line.len() {
                let inner = line[end..].chars().next().expect("end in bounds");
                token.push(inner);
                end += inner.len_utf8();
                if inner == '`' {
                    break;
                }
            }
            continue;
        }
        if matches!(ch, '"' | '\'') {
            let quote = ch;
            token.push(ch);
            end += ch.len_utf8();
            let mut inner_escaped = false;
            while end < line.len() {
                let inner = line[end..].chars().next().expect("end in bounds");
                token.push(inner);
                end += inner.len_utf8();
                if inner_escaped {
                    inner_escaped = false;
                    continue;
                }
                if inner == '\\' && quote == '"' {
                    inner_escaped = true;
                    continue;
                }
                if inner == quote {
                    break;
                }
            }
            continue;
        }
        token.push(ch);
        end += ch.len_utf8();
    }
    end
}

fn consume_balanced_shell_group(
    line: &str,
    start: usize,
    open: char,
    close: char,
    token: &mut String,
) -> usize {
    let mut end = start;
    let mut depth = 0usize;
    let mut escaped = false;
    let mut quote: Option<char> = None;
    while end < line.len() {
        let ch = line[end..].chars().next().expect("end in bounds");
        token.push(ch);
        end += ch.len_utf8();
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '"' | '\'') {
            quote = Some(ch);
            continue;
        }
        if ch == open {
            depth += 1;
            continue;
        }
        if ch == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
    end
}

fn is_shell_word_break(ch: char) -> bool {
    // Commas and closing brackets are ordinary glued shell-word characters
    // (`token=abc,def`, `password abc,def]`), so they must not truncate credentials.
    ch.is_whitespace() || matches!(ch, ';' | '}' | '&')
}

/// Consume one leading non-whitespace shell control character so rebuilds keep
/// operators like `&&` / `&` when a pending sensitive argument is missing.
fn take_leading_shell_operator(line: &str) -> Option<(&str, &str)> {
    let ch = line.chars().next()?;
    if ch.is_whitespace() || !is_shell_word_break(ch) {
        return None;
    }
    let len = ch.len_utf8();
    Some((&line[..len], &line[len..]))
}

pub(super) fn redact_inline_sensitive_assignments(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut scan_cursor = 0usize;
    let mut output_cursor = 0usize;
    while let Some((separator, ch)) = find_next_assignment_separator(line, scan_cursor) {
        let Some(key_start) = assignment_key_start(line, separator) else {
            scan_cursor = separator + ch.len_utf8();
            continue;
        };
        let key = &line[key_start..separator];
        if !is_sensitive_key(key) && !is_sensitive_option_key(key) {
            scan_cursor = separator + ch.len_utf8();
            continue;
        }

        output.push_str(&line[output_cursor..separator + ch.len_utf8()]);
        let (prefix_end, value_end) =
            sensitive_assignment_value_bounds(line, separator + ch.len_utf8(), key, ch);
        output.push_str(&line[separator + ch.len_utf8()..prefix_end]);
        output.push_str("[REDACTED]");
        scan_cursor = value_end;
        output_cursor = value_end;
    }

    if output_cursor == 0 {
        return line.to_string();
    }
    output.push_str(&line[output_cursor..]);
    output
}

pub(super) fn contains_inline_sensitive_assignment(line: &str) -> bool {
    let mut scan_cursor = 0usize;
    while let Some((separator, ch)) = find_next_assignment_separator(line, scan_cursor) {
        let Some(key_start) = assignment_key_start(line, separator) else {
            scan_cursor = separator + ch.len_utf8();
            continue;
        };
        let key = &line[key_start..separator];
        if is_sensitive_key(key) || is_sensitive_option_key(key) {
            return true;
        }
        scan_cursor = separator + ch.len_utf8();
    }
    false
}

fn find_next_assignment_separator(line: &str, cursor: usize) -> Option<(usize, char)> {
    line[cursor..]
        .char_indices()
        .find_map(|(offset, ch)| matches!(ch, '=' | ':').then_some((cursor + offset, ch)))
}

fn assignment_key_start(line: &str, separator: usize) -> Option<usize> {
    let mut key_end = separator;
    while let Some((idx, ch)) = line[..key_end].char_indices().next_back() {
        // Match token splitting: Unicode whitespace (e.g. NBSP) may separate the key
        // from `=` / `:` and must be skipped before reading the key characters.
        if ch.is_whitespace() || matches!(ch, '"' | '\'' | '`') {
            key_end = idx;
            continue;
        }
        break;
    }

    let mut start = key_end;
    for (idx, ch) in line[..key_end].char_indices().rev() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            start = idx;
            continue;
        }
        break;
    }
    (start < key_end).then_some(start)
}

fn sensitive_assignment_value_bounds(
    line: &str,
    value_offset: usize,
    key: &str,
    separator: char,
) -> (usize, usize) {
    let prefix_end = skip_unicode_whitespace(line, value_offset);
    if key_owns_header_value(key, separator) {
        return (prefix_end, header_sensitive_value_end(line, prefix_end));
    }

    let Some((token, remaining)) = take_shell_like_argument(&line[prefix_end..]) else {
        return (prefix_end, prefix_end);
    };
    let mut value_end = line.len() - remaining.len();

    // `token=Bearer xyz` owns both words; shell parsing alone would leave xyz.
    if token.eq_ignore_ascii_case("bearer")
        || token
            .trim_matches(|ch: char| matches!(ch, '"' | '\''))
            .eq_ignore_ascii_case("bearer")
    {
        let second_start = skip_unicode_whitespace(line, value_end);
        if let Some((_, second_remaining)) = take_shell_like_argument(&line[second_start..]) {
            value_end = line.len() - second_remaining.len();
        }
    }

    // Prefer redacting inside quotes when the shell word opens with a quote.
    // Keep glued unquoted suffixes (`"abc"tail`) inside the credential, but stop
    // before a comma/bracket that starts a new field (`"secret","safe":…`).
    if let Some(quote) = token.chars().next().filter(|ch| matches!(ch, '"' | '\'')) {
        let value_start = prefix_end + quote.len_utf8();
        let close = quoted_sensitive_value_end(line, value_start, quote);
        if close >= value_start && close <= value_end && line[close..].starts_with(quote) {
            let after_close = close + quote.len_utf8();
            if after_close >= value_end {
                return (value_start, close);
            }
            let rest = &line[after_close..value_end];
            if rest.starts_with(',') || rest.starts_with(']') {
                return (value_start, close);
            }
            return (prefix_end, value_end);
        }
    }

    (prefix_end, value_end)
}

fn quoted_sensitive_value_end(line: &str, value_start: usize, quote: char) -> usize {
    let mut escaped = false;
    for (offset, ch) in line[value_start..].char_indices() {
        if ch == quote && !escaped {
            return value_start + offset;
        }
        escaped = ch == '\\' && !escaped;
        if ch != '\\' {
            escaped = false;
        }
    }
    line.len()
}

fn header_sensitive_value_end(line: &str, value_start: usize) -> usize {
    let mut idx = value_start;
    let mut escaped = false;
    while idx < line.len() {
        let ch = line[idx..].chars().next().expect("idx in bounds");
        if escaped {
            escaped = false;
            idx += ch.len_utf8();
            continue;
        }
        if ch == '\\' {
            // Track shell-style escapes so `\"` inside a double-quoted header
            // does not terminate the Authorization/Cookie span early.
            escaped = true;
            idx += ch.len_utf8();
            continue;
        }
        if matches!(ch, '`' | '\r' | '\n') {
            return idx;
        }
        if matches!(ch, '"' | '\'') {
            // Consume quotes only when they open a parameter value after `=`,
            // e.g. `Cookie: session="abc123"`. A trailing shell closer such as
            // `...csrf=short'` must still terminate the header span.
            let opens_parameter =
                line[..idx].chars().rev().find(|c| !c.is_ascii_whitespace()) == Some('=');
            if opens_parameter {
                let value_body = idx + ch.len_utf8();
                let close = quoted_sensitive_value_end(line, value_body, ch);
                idx = if close < line.len() && line[close..].starts_with(ch) {
                    close + ch.len_utf8()
                } else {
                    close
                };
                continue;
            }
            return idx;
        }
        idx += ch.len_utf8();
    }
    line.len()
}

pub(super) fn skip_unicode_whitespace(line: &str, offset: usize) -> usize {
    line[offset..]
        .char_indices()
        .find_map(|(relative, ch)| (!ch.is_whitespace()).then_some(offset + relative))
        .unwrap_or(line.len())
}

pub(super) fn split_sensitive_assignment(line: &str) -> Option<(&str, &str)> {
    let (idx, separator_len) = line
        .find('=')
        .map(|idx| (idx, 1))
        .or_else(|| line.find(':').map(|idx| (idx, 1)))?;
    let key = line[..idx].trim();
    if !is_sensitive_key(key) {
        return None;
    }
    Some((&line[..idx + separator_len], &line[idx + separator_len..]))
}

fn token_expects_sensitive_argument(token: &str) -> bool {
    // Attached forms such as `-ualice:pw` already carry the credential; do not
    // also treat the next whitespace token as a secret argument.
    if redact_attached_sensitive_short_option(token).is_some() {
        return false;
    }
    let option =
        token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
    let Some(option) = option
        .strip_prefix("--")
        .or_else(|| option.strip_prefix('-'))
    else {
        return false;
    };
    if option.is_empty() {
        return false;
    }
    !option.contains('=') && is_sensitive_option_key(option)
}

/// True when the next token is an attached sensitive short option whose value
/// opens with a quote, e.g. `-u"alice:correct horse"`.
fn attached_sensitive_short_option_starts_quoted(token: &str) -> bool {
    if !token.starts_with('-') || token.starts_with("--") {
        return false;
    }
    let after_dash = &token[1..];
    let mut chars = after_dash.chars();
    let Some(opt) = chars.next() else {
        return false;
    };
    if !opt.is_ascii_alphabetic() || !is_sensitive_option_key(&opt.to_string()) {
        return false;
    }
    matches!(chars.next(), Some('"' | '\''))
}

/// Recognize curl-style attached short options such as `-ualice:pw`.
///
/// Only single-letter sensitive options are considered; the remainder of the
/// token is treated as the credential value.
fn redact_attached_sensitive_short_option(token: &str) -> Option<String> {
    let leading_len = token
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)?;
    let core = &token[leading_len..];
    if !core.starts_with('-') || core.starts_with("--") {
        return None;
    }
    let after_dash = &core[1..];
    let mut chars = after_dash.chars();
    let opt = chars.next()?;
    if !opt.is_ascii_alphabetic() {
        return None;
    }
    let value = chars.as_str();
    if value.is_empty() {
        return None;
    }
    // Clustered flags (`-vu`) and prose (`-username`) stay untouched. Attached
    // credentials usually contain punctuation (`:`), digits, or other marks.
    if value.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    if !is_sensitive_option_key(&opt.to_string()) {
        return None;
    }
    Some(format!("{}-{}[REDACTED]", &token[..leading_len], opt))
}
