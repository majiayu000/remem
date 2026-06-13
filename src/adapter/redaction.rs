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

fn redact_sensitive_line(line: &str) -> String {
    if let Some((prefix, _)) = split_sensitive_assignment(line) {
        return format!("{prefix}[REDACTED]");
    }
    redact_sensitive_tokens(line)
}

fn redact_hook_payload_text(text: &str) -> String {
    text.lines()
        .map(redact_hook_payload_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_hook_payload_line(line: &str) -> String {
    if let Some((prefix, _)) = split_sensitive_assignment(line) {
        return format!("{prefix}[REDACTED]");
    }
    let line = redact_inline_sensitive_assignments(line);
    redact_sensitive_tokens(&line)
}

fn redact_sensitive_tokens(line: &str) -> String {
    let mut previous_was_bearer = false;
    let mut previous_was_sensitive_option = false;
    line.split_whitespace()
        .map(|token| {
            let redacted = if previous_was_bearer || previous_was_sensitive_option {
                "[REDACTED]"
            } else {
                redact_token(token)
            };
            previous_was_sensitive_option = token_expects_sensitive_argument(token);
            previous_was_bearer = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .eq_ignore_ascii_case("bearer");
            redacted
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    line[value_start..]
        .char_indices()
        .find_map(|(relative, ch)| {
            matches!(ch, '"' | '\'' | '`' | '\r' | '\n').then_some(value_start + relative)
        })
        .unwrap_or(line.len())
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
    )
}

fn token_expects_sensitive_argument(token: &str) -> bool {
    let option =
        token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
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

pub(crate) fn redact_token(token: &str) -> &str {
    let trimmed =
        token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
    if contains_prefixed_secret(trimmed)
        || (trimmed.len() >= 32
            && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
            && trimmed.chars().any(|ch| ch.is_ascii_digit()))
    {
        "[REDACTED]"
    } else {
        token
    }
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
