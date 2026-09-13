use crate::db;

pub(crate) const HOOK_PAYLOAD_PREVIEW_REDACTION_LOOKAHEAD_BYTES: usize = 4 * 1024;

pub(crate) fn redact_and_truncate(text: &str, max_bytes: usize) -> String {
    let redacted = redact_sensitive_text(text);
    db::truncate_str(&redacted, max_bytes).to_string()
}

pub(crate) fn redact_hook_payload_preview(raw_payload: &str, max_bytes: usize) -> String {
    let preview_input = hook_payload_preview_redaction_input(raw_payload, max_bytes);
    let redacted = serde_json::from_str::<serde_json::Value>(preview_input)
        .map(|value| redact_hook_payload_value(&value).to_string())
        .unwrap_or_else(|_| redact_hook_payload_text(preview_input));
    db::truncate_str(&redacted, max_bytes).to_string()
}

pub(crate) fn hook_payload_preview_contains_sensitive_match(
    raw_payload: &str,
    max_bytes: usize,
) -> bool {
    let preview_input = hook_payload_preview_redaction_input(raw_payload, max_bytes);
    serde_json::from_str::<serde_json::Value>(preview_input)
        .map(|value| hook_payload_value_contains_sensitive_match(&value))
        .unwrap_or_else(|_| hook_payload_text_contains_sensitive_match(preview_input))
}

pub(crate) fn hook_payload_preview_redaction_input(raw_payload: &str, max_bytes: usize) -> &str {
    db::truncate_str(
        raw_payload,
        max_bytes.saturating_add(HOOK_PAYLOAD_PREVIEW_REDACTION_LOOKAHEAD_BYTES),
    )
}

fn redact_hook_payload_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_key(key) {
                        serde_json::Value::String("[REDACTED]".to_string())
                    } else {
                        redact_hook_payload_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(redact_hook_payload_value)
                .collect::<Vec<_>>(),
        ),
        serde_json::Value::String(text) => {
            serde_json::Value::String(redact_hook_payload_text(text))
        }
        _ => value.clone(),
    }
}

fn hook_payload_value_contains_sensitive_match(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            is_sensitive_key(key) || hook_payload_value_contains_sensitive_match(value)
        }),
        serde_json::Value::Array(items) => items
            .iter()
            .any(hook_payload_value_contains_sensitive_match),
        serde_json::Value::String(text) => hook_payload_text_contains_sensitive_match(text),
        _ => false,
    }
}

pub(crate) fn redact_sensitive_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_key(key) {
                        serde_json::Value::String("[REDACTED]".to_string())
                    } else {
                        redact_sensitive_value(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_sensitive_value).collect::<Vec<_>>())
        }
        serde_json::Value::String(text) => serde_json::Value::String(redact_sensitive_text(text)),
        _ => value.clone(),
    }
}

pub(crate) fn redact_sensitive_text(text: &str) -> String {
    text.lines()
        .map(redact_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Redact user-facing projection text (MCP/CLI) including short inline
/// credential assignments such as `Investigate token=abc123` and space-separated
/// sensitive option arguments such as `curl --oauth2-bearer tiny-token`.
///
/// Keep this separate from [`redact_sensitive_text`], which intentionally omits
/// the hook inline-assignment and sensitive-option heuristics to avoid scrubbing
/// ordinary code/prose.
pub(crate) fn redact_projected_sensitive_text(text: &str) -> String {
    let redacted = redact_inline_sensitive_assignments(text);
    redacted
        .lines()
        .map(redact_projected_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_projected_sensitive_line(line: &str) -> String {
    if let Some((prefix, value)) = split_sensitive_assignment(line) {
        return redact_assignment_keeping_suffix(prefix, value);
    }
    redact_tokens(line, true, true)
}

fn redact_sensitive_line(line: &str) -> String {
    if let Some((prefix, _)) = split_sensitive_assignment(line) {
        return format!("{prefix}[REDACTED]");
    }
    redact_tokens(line, false, false)
}

fn redact_hook_payload_text(text: &str) -> String {
    let redacted = redact_inline_sensitive_assignments(text);
    redacted
        .lines()
        .map(redact_hook_payload_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn hook_payload_text_contains_sensitive_match(text: &str) -> bool {
    contains_inline_sensitive_assignment(text)
        || text.lines().any(hook_payload_line_contains_sensitive_match)
}

fn redact_hook_payload_line(line: &str) -> String {
    if let Some((prefix, value)) = split_sensitive_assignment(line) {
        return redact_assignment_keeping_suffix(prefix, value);
    }
    redact_tokens(line, true, false)
}

fn hook_payload_line_contains_sensitive_match(line: &str) -> bool {
    split_sensitive_assignment(line).is_some() || tokens_contain_sensitive_match(line, true)
}

/// After the bounded inline pass, a line may look like
/// `token=[REDACTED] investigate database regression` or
/// `token="[REDACTED]" investigate database regression`. Preserve the benign
/// suffix instead of treating the whole remainder as the credential value.
fn redact_assignment_keeping_suffix(prefix: &str, value: &str) -> String {
    let trimmed = value.trim_start();
    if let Some(rest) = strip_redacted_assignment_marker(trimmed) {
        return format!("{prefix}[REDACTED]{rest}");
    }
    format!("{prefix}[REDACTED]")
}

fn strip_redacted_assignment_marker(value: &str) -> Option<&str> {
    if let Some(rest) = value.strip_prefix("[REDACTED]") {
        // Only treat whitespace-delimited remainders as benign suffixes. A glued
        // remainder such as `[REDACTED]"abc123"` is leftover secret material.
        if rest.is_empty() || rest.starts_with(|ch: char| ch.is_ascii_whitespace()) {
            return Some(rest);
        }
        return None;
    }
    for quote in ['"', '\''] {
        let mut chars = value.chars();
        if chars.next() != Some(quote) {
            continue;
        }
        let after_open = chars.as_str();
        let Some(after_marker) = after_open.strip_prefix("[REDACTED]") else {
            continue;
        };
        let mut after_marker_chars = after_marker.chars();
        if after_marker_chars.next() != Some(quote) {
            continue;
        }
        return Some(after_marker_chars.as_str());
    }
    None
}

fn redact_tokens(
    line: &str,
    redact_sensitive_options: bool,
    preserve_filesystem_paths: bool,
) -> String {
    let mut previous_was_bearer = false;
    let mut previous_was_sensitive_option = false;
    let mut redacted_tokens = Vec::new();
    let mut rest = line;

    while let Some((token, remaining)) =
        next_redaction_token(rest, previous_was_bearer || previous_was_sensitive_option)
    {
        let redacted = if previous_was_bearer || previous_was_sensitive_option {
            "[REDACTED]".to_string()
        } else {
            redact_token_with_options(&token, preserve_filesystem_paths)
        };
        previous_was_sensitive_option =
            redact_sensitive_options && token_expects_sensitive_argument(&token);
        previous_was_bearer = token
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
            .eq_ignore_ascii_case("bearer");
        redacted_tokens.push(redacted);
        rest = remaining;
    }

    redacted_tokens.join(" ")
}

fn tokens_contain_sensitive_match(line: &str, redact_sensitive_options: bool) -> bool {
    let mut previous_was_bearer = false;
    let mut previous_was_sensitive_option = false;
    let mut rest = line;
    while let Some((token, remaining)) =
        next_redaction_token(rest, previous_was_bearer || previous_was_sensitive_option)
    {
        if previous_was_bearer || previous_was_sensitive_option || redact_token(&token) != token {
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
/// Bearer argument so benign phrases like `Review "phase 2 migration …"` are
/// not glued into one length-heuristic hit.
fn next_redaction_token(line: &str, group_quotes: bool) -> Option<(String, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    if group_quotes {
        take_shell_like_argument(trimmed)
    } else {
        take_whitespace_token(trimmed)
    }
}

fn take_whitespace_token(line: &str) -> Option<(String, &str)> {
    let end = line
        .char_indices()
        .find_map(|(idx, ch)| ch.is_ascii_whitespace().then_some(idx))
        .unwrap_or(line.len());
    Some((line[..end].to_string(), &line[end..]))
}

/// Consume one shell-like argument, keeping a quoted span intact so
/// `-u "alice:correct horse"` redacts as a single value.
fn take_shell_like_argument(line: &str) -> Option<(String, &str)> {
    let mut chars = line.chars();
    let first = chars.next()?;
    if matches!(first, '"' | '\'') {
        let mut token = String::from(first);
        let quote = first;
        let mut escaped = false;
        for ch in chars.by_ref() {
            token.push(ch);
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote == '"' {
                escaped = true;
                continue;
            }
            if ch == quote {
                break;
            }
        }
        return Some((token, chars.as_str()));
    }

    let end = line
        .char_indices()
        .find_map(|(idx, ch)| ch.is_ascii_whitespace().then_some(idx))
        .unwrap_or(line.len());
    Some((line[..end].to_string(), &line[end..]))
}

fn redact_inline_sensitive_assignments(line: &str) -> String {
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

fn contains_inline_sensitive_assignment(line: &str) -> bool {
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
        if ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | '`') {
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
    let prefix_end = skip_ascii_whitespace(line, value_offset);
    let Some((quote_offset, quote)) = line[prefix_end..]
        .char_indices()
        .next()
        .filter(|(_, ch)| matches!(ch, '"' | '\''))
        .map(|(offset, ch)| (prefix_end + offset, ch))
    else {
        let value_end = if key_owns_header_value(key, separator) {
            header_sensitive_value_end(line, prefix_end)
        } else {
            unquoted_sensitive_value_end(line, prefix_end)
        };
        return (prefix_end, value_end);
    };

    let value_start = quote_offset + quote.len_utf8();
    (
        value_start,
        quoted_sensitive_value_end(line, value_start, quote),
    )
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
    while idx < line.len() {
        let ch = line[idx..].chars().next().expect("idx in bounds");
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

fn skip_ascii_whitespace(line: &str, offset: usize) -> usize {
    line[offset..]
        .char_indices()
        .find_map(|(relative, ch)| (!ch.is_ascii_whitespace()).then_some(offset + relative))
        .unwrap_or(line.len())
}

fn unquoted_sensitive_value_end(line: &str, value_start: usize) -> usize {
    let first_end = line[value_start..]
        .char_indices()
        .find_map(|(relative, ch)| {
            (ch.is_ascii_whitespace() || matches!(ch, ',' | ';' | '}' | ']' | '&'))
                .then_some(value_start + relative)
        })
        .unwrap_or(line.len());
    if !line[value_start..first_end].eq_ignore_ascii_case("bearer") {
        return first_end;
    }
    let second_start = skip_ascii_whitespace(line, first_end);
    line[second_start..]
        .char_indices()
        .find_map(|(relative, ch)| {
            (ch.is_ascii_whitespace() || matches!(ch, ',' | ';' | '}' | ']' | '&'))
                .then_some(second_start + relative)
        })
        .unwrap_or(line.len())
}

fn split_sensitive_assignment(line: &str) -> Option<(&str, &str)> {
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

fn normalized_sensitive_key(key: &str) -> String {
    key.trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .to_ascii_lowercase()
        .replace('-', "_")
}

fn is_sensitive_option_key(key: &str) -> bool {
    matches!(
        normalized_sensitive_key(key).as_str(),
        "u" | "user" | "pass" | "oauth2_bearer" | "proxy_user" | "proxy_pass"
    ) || is_sensitive_key(key)
}

fn token_expects_sensitive_argument(token: &str) -> bool {
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

fn key_owns_header_value(key: &str, separator: char) -> bool {
    let normalized = normalized_sensitive_key(key);
    matches!(normalized.as_str(), "cookie" | "set_cookie")
        || (separator == ':' && matches!(normalized.as_str(), "auth" | "authorization"))
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = normalized_sensitive_key(key);
    matches!(
        normalized.as_str(),
        "api_key"
            | "apikey"
            | "auth"
            | "authorization"
            | "bearer"
            | "cookie"
            | "set_cookie"
            | "password"
            | "passwd"
            | "secret"
            | "token"
            | "access_token"
            | "accesstoken"
            | "refresh_token"
            | "refreshtoken"
            | "id_token"
            | "idtoken"
            | "client_secret"
            | "clientsecret"
            | "private_key"
            | "privatekey"
    ) || normalized.ends_with("_api_key")
        || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
}

pub(crate) fn redact_token(token: &str) -> String {
    redact_token_with_options(token, false)
}

fn redact_token_with_options(token: &str, preserve_filesystem_paths: bool) -> String {
    let trimmed =
        token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
    if let Some(redacted) = redact_url_userinfo(token) {
        redacted
    } else if contains_prefixed_secret(trimmed)
        || (!(preserve_filesystem_paths && looks_like_filesystem_path(token))
            && trimmed.len() >= 32
            && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
            && trimmed.chars().any(|ch| ch.is_ascii_digit()))
    {
        "[REDACTED]".to_string()
    } else {
        token.to_string()
    }
}

fn looks_like_filesystem_path(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';' | ')'));
    trimmed.starts_with('/')
        || trimmed.starts_with("~/")
        || trimmed.starts_with("./")
        || trimmed.starts_with(".\\")
        || (trimmed.len() >= 3
            && trimmed.as_bytes()[1] == b':'
            && trimmed.as_bytes()[0].is_ascii_alphabetic()
            && matches!(trimmed.as_bytes()[2], b'\\' | b'/'))
}

fn redact_url_userinfo(token: &str) -> Option<String> {
    let scheme_end = token.find("://")?;
    let authority_start = scheme_end + 3;
    let authority = &token[authority_start..];
    let at = authority.find('@')?;
    let authority_end = authority
        .char_indices()
        .find_map(|(idx, ch)| matches!(ch, '/' | '?' | '#').then_some(idx))
        .unwrap_or(authority.len());
    if at == 0 || at >= authority_end {
        return None;
    }

    Some(format!(
        "{}[REDACTED]{}",
        &token[..authority_start],
        &authority[at..]
    ))
}

fn contains_prefixed_secret(token: &str) -> bool {
    [("sk-", 8), ("ghp_", 8), ("github_pat_", 4), ("xoxb-", 8)]
        .iter()
        .any(|(prefix, min_suffix_len)| {
            contains_prefixed_secret_with(token, prefix, *min_suffix_len)
        })
}

fn contains_prefixed_secret_with(token: &str, prefix: &str, min_suffix_len: usize) -> bool {
    token.match_indices(prefix).any(|(index, _)| {
        has_secret_prefix_boundary(token, index)
            && key_like_suffix_len(&token[index + prefix.len()..]) >= min_suffix_len
    })
}

fn has_secret_prefix_boundary(token: &str, index: usize) -> bool {
    index == 0
        || token[..index]
            .chars()
            .next_back()
            .is_some_and(|ch| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
}

fn key_like_suffix_len(suffix: &str) -> usize {
    suffix
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .count()
}
