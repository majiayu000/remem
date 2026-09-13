//! Sensitive-key classification and single-token redaction heuristics.

pub(super) fn redact_token(token: &str) -> String {
    redact_token_with_options(token, false)
}

pub(super) fn redact_token_with_options(token: &str, preserve_filesystem_paths: bool) -> String {
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

pub(super) fn normalized_sensitive_key(key: &str) -> String {
    key.trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .to_ascii_lowercase()
        .replace('-', "_")
}

pub(super) fn is_sensitive_option_key(key: &str) -> bool {
    matches!(
        normalized_sensitive_key(key).as_str(),
        "u" | "user" | "pass" | "oauth2_bearer" | "proxy_user" | "proxy_pass"
    ) || is_sensitive_key(key)
}

pub(super) fn key_owns_header_value(key: &str, separator: char) -> bool {
    let normalized = normalized_sensitive_key(key);
    matches!(normalized.as_str(), "cookie" | "set_cookie")
        || (separator == ':' && matches!(normalized.as_str(), "auth" | "authorization"))
}

pub(super) fn is_sensitive_key(key: &str) -> bool {
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
            | "access_key"
            | "secret_access_key"
    ) || normalized.ends_with("_api_key")
        || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
        || normalized.ends_with("_access_key")
}

pub(super) fn looks_like_filesystem_path(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | ',' | ';' | ')'));
    trimmed.starts_with('/')
        || trimmed.starts_with("~/")
        || trimmed.starts_with("./")
        || trimmed.starts_with(".\\")
        // UNC (`\\server\share\...`) and extended-length (`\\?\...`, `\\.\...`)
        // Windows prefixes; also accept `//server/share` style UNC.
        || trimmed.starts_with("\\\\")
        || trimmed.starts_with("//")
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
