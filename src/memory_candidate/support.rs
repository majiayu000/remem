const MIN_SUPPORT_TOKEN_OVERLAP: usize = 6;
const MIN_SUPPORT_TOKEN_RATIO: f64 = 0.72;
const MAX_SUPPORT_TOKEN_WINDOW_EXTRA: usize = 5;
const SUPPORT_TOKEN_MIN_CHARS: usize = 4;

#[rustfmt::skip]
const SUPPORT_RISK_TOKENS: &[&str] = &[
    "allow", "allowed", "allows", "cannot", "cant", "could", "couldn", "delete", "deleted",
    "deletes", "deny", "denied", "denies", "didn", "disable", "disabled", "disables", "doesn", "don",
    "enable", "enabled", "enables", "fail", "failed", "failing", "fails", "hadn", "hasn",
    "haven", "if", "ignore", "ignored", "ignores", "isn", "may", "might", "must", "never",
    "no", "not", "prevent", "prevented", "prevents", "reject", "rejected", "rejects",
    "remove", "removed", "removes", "should", "shouldn", "skip", "skipped", "skips",
    "succeed", "succeeded", "succeeds", "success", "unless", "wasn", "weren", "without",
    "won", "wouldn",
];

pub(super) fn has_conservative_support_token_overlap(
    candidate_text: &str,
    observation_text: &str,
) -> bool {
    if contains_support_risk_token(candidate_text) || contains_support_risk_token(observation_text)
    {
        return false;
    }
    let candidate_tokens = support_tokens(candidate_text);
    if candidate_tokens.len() < MIN_SUPPORT_TOKEN_OVERLAP {
        return false;
    }
    support_token_segments(observation_text)
        .into_iter()
        .any(|observation_tokens| {
            let Some((matched, start, end)) =
                ordered_support_window(&candidate_tokens, &observation_tokens)
            else {
                return false;
            };
            let window_len = end.saturating_sub(start) + 1;
            matched >= MIN_SUPPORT_TOKEN_OVERLAP
                && (matched as f64 / candidate_tokens.len() as f64) >= MIN_SUPPORT_TOKEN_RATIO
                && window_len <= candidate_tokens.len() + MAX_SUPPORT_TOKEN_WINDOW_EXTRA
        })
}

fn ordered_support_window(
    candidate_tokens: &[String],
    observation_tokens: &[String],
) -> Option<(usize, usize, usize)> {
    let first_candidate = candidate_tokens.first()?;
    let mut best = None;
    for (candidate_start, observation) in observation_tokens.iter().enumerate() {
        if observation != first_candidate {
            continue;
        }
        let mut end = candidate_start;
        let mut matched = 1;
        let mut search_from = candidate_start + 1;
        for candidate in &candidate_tokens[1..] {
            let Some(position) = observation_tokens
                .iter()
                .enumerate()
                .skip(search_from)
                .find_map(|(index, observation)| (observation == candidate).then_some(index))
            else {
                matched = 0;
                break;
            };
            end = position;
            matched += 1;
            search_from = position + 1;
        }
        if matched == candidate_tokens.len()
            && best
                .map(|(_, best_start, best_end)| end - candidate_start < best_end - best_start)
                .unwrap_or(true)
        {
            best = Some((matched, candidate_start, end));
        }
    }
    best
}

fn support_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.chars().count() >= SUPPORT_TOKEN_MIN_CHARS)
        .filter(|token| !is_support_stop_token(token))
        .map(normalize_support_token)
        .collect()
}

fn support_token_segments(text: &str) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut token = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            token.push(ch.to_ascii_lowercase());
            continue;
        }
        flush_support_segment_token(&mut token, &mut current, &mut segments);
        if is_support_clause_boundary_char(ch) {
            finish_support_segment(&mut current, &mut segments);
        }
    }
    flush_support_segment_token(&mut token, &mut current, &mut segments);
    finish_support_segment(&mut current, &mut segments);
    segments
}

fn flush_support_segment_token(
    token: &mut String,
    current: &mut Vec<String>,
    segments: &mut Vec<Vec<String>>,
) {
    if token.is_empty() {
        return;
    }
    if is_support_clause_boundary_token(token) {
        finish_support_segment(current, segments);
    } else if token.chars().count() >= SUPPORT_TOKEN_MIN_CHARS && !is_support_stop_token(token) {
        current.push(normalize_support_token(token));
    }
    token.clear();
}

fn finish_support_segment(current: &mut Vec<String>, segments: &mut Vec<Vec<String>>) {
    if !current.is_empty() {
        segments.push(std::mem::take(current));
    }
}

fn is_support_clause_boundary_char(ch: char) -> bool {
    matches!(ch, '.' | ';' | ':' | '?' | '!')
}

fn is_support_clause_boundary_token(token: &str) -> bool {
    matches!(
        token,
        "although" | "and" | "but" | "however" | "though" | "whereas" | "while"
    )
}

fn normalize_support_token(token: &str) -> String {
    if let Some(stem) = token.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if token.len() > 4 && token.ends_with('s') && !token.ends_with("ss") && !token.ends_with("us") {
        return token[..token.len() - 1].to_string();
    }
    token.to_string()
}

fn contains_support_risk_token(text: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| SUPPORT_RISK_TOKENS.contains(&token))
}

fn is_support_stop_token(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "also"
            | "from"
            | "into"
            | "only"
            | "over"
            | "that"
            | "their"
            | "then"
            | "this"
            | "uses"
            | "with"
    )
}
