const MIN_SUPPORT_TOKEN_OVERLAP: usize = 6;
const MIN_SUPPORT_TOKEN_RATIO: f64 = 0.72;
const MAX_SUPPORT_TOKEN_WINDOW_EXTRA: usize = 5;
const SUPPORT_TOKEN_MIN_CHARS: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
struct SupportToken {
    text: String,
    required: bool,
}

#[derive(Clone, Copy)]
struct SupportWindow {
    matched: usize,
    required_matched: usize,
    start: usize,
    end: usize,
}

#[rustfmt::skip]
const SUPPORT_RISK_TOKENS: &[&str] = &[
    "allow", "allowed", "allows", "cannot", "cant", "could", "couldn", "delete", "deleted",
    "deletes", "deny", "denied", "denies", "didn", "disable", "disabled", "disables", "doesn", "don",
    "enable", "enabled", "enables", "fail", "failed", "failing", "fails", "failure", "failures", "hadn", "hasn",
    "haven", "if", "ignore", "ignored", "ignores", "isn", "may", "might", "must", "never",
    "no", "not", "pass", "passed", "passes", "passing", "plan", "planned", "planning", "plans",
    "prevent", "prevented", "prevents", "reject", "rejected", "rejects",
    "remove", "removed", "removes", "shall", "should", "shouldn", "skip", "skipped", "skips",
    "succeed", "succeeded", "succeeds", "success", "unless", "wasn", "weren", "will", "without",
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
    let candidate_required = candidate_tokens
        .iter()
        .filter(|token| token.required)
        .count();
    support_token_segments(observation_text)
        .into_iter()
        .any(|observation_tokens| {
            ordered_support_window(&candidate_tokens, &observation_tokens).is_some_and(|window| {
                window.matched >= MIN_SUPPORT_TOKEN_OVERLAP
                    && window.required_matched == candidate_required
                    && (window.matched as f64 / candidate_tokens.len() as f64)
                        >= MIN_SUPPORT_TOKEN_RATIO
            })
        })
}

fn ordered_support_window(
    candidate_tokens: &[SupportToken],
    observation_tokens: &[SupportToken],
) -> Option<SupportWindow> {
    let first_candidate = candidate_tokens.first()?;
    let mut best = None;
    for (candidate_start, observation) in observation_tokens.iter().enumerate() {
        if observation.text != first_candidate.text {
            continue;
        }
        let mut end = candidate_start;
        let mut matched = 1;
        let mut required_matched = usize::from(first_candidate.required);
        let mut search_from = candidate_start + 1;
        for candidate in &candidate_tokens[1..] {
            let Some(position) = observation_tokens
                .iter()
                .enumerate()
                .skip(search_from)
                .find_map(|(index, observation)| {
                    (observation.text == candidate.text).then_some(index)
                })
            else {
                continue;
            };
            end = position;
            matched += 1;
            if candidate.required {
                required_matched += 1;
            }
            search_from = position + 1;
        }
        let window_len = end.saturating_sub(candidate_start) + 1;
        if window_len <= candidate_tokens.len() + MAX_SUPPORT_TOKEN_WINDOW_EXTRA
            && best
                .map(|best| is_better_support_window(matched, candidate_start, end, best))
                .unwrap_or(true)
        {
            best = Some(SupportWindow {
                matched,
                required_matched,
                start: candidate_start,
                end,
            });
        }
    }
    best
}

fn is_better_support_window(matched: usize, start: usize, end: usize, best: SupportWindow) -> bool {
    matched > best.matched || (matched == best.matched && end - start < best.end - best.start)
}

fn support_tokens(text: &str) -> Vec<SupportToken> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(support_token)
        .collect()
}

fn support_token_segments(text: &str) -> Vec<Vec<SupportToken>> {
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
    current: &mut Vec<SupportToken>,
    segments: &mut Vec<Vec<SupportToken>>,
) {
    if token.is_empty() {
        return;
    }
    if is_support_clause_boundary_token(token) {
        finish_support_segment(current, segments);
    } else if let Some(token) = support_token(token) {
        current.push(token);
    }
    token.clear();
}

fn finish_support_segment(current: &mut Vec<SupportToken>, segments: &mut Vec<Vec<SupportToken>>) {
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
        "after"
            | "although"
            | "and"
            | "as"
            | "because"
            | "before"
            | "but"
            | "however"
            | "once"
            | "since"
            | "then"
            | "though"
            | "until"
            | "when"
            | "whereas"
            | "while"
    )
}

fn support_token(token: &str) -> Option<SupportToken> {
    if is_support_stop_token(token) {
        return None;
    }
    let required_identifier = is_required_support_identifier(token);
    if !required_identifier && token.chars().count() < SUPPORT_TOKEN_MIN_CHARS {
        return None;
    }
    let text = normalize_support_token(token);
    Some(SupportToken {
        required: required_identifier || !is_optional_support_token(&text),
        text,
    })
}

fn is_required_support_identifier(token: &str) -> bool {
    matches!(
        token,
        "api" | "cli" | "db" | "jwt" | "llm" | "mcp" | "sql" | "ssh" | "ssl" | "tls" | "ui"
    )
}

fn is_optional_support_token(token: &str) -> bool {
    matches!(token, "review")
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
