const MIN_SUPPORT_TOKEN_OVERLAP: usize = 6;
const MIN_SUPPORT_TOKEN_RATIO: f64 = 0.72;
const MAX_SUPPORT_TOKEN_WINDOW_EXTRA: usize = 5;
const SUPPORT_TOKEN_MIN_CHARS: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
struct SupportToken {
    text: String,
    required: bool,
}

#[rustfmt::skip]
const SUPPORT_RISK_TOKENS: &[&str] = &[
    "allow", "allowed", "allows", "cannot", "cant", "could", "couldn", "delete", "deleted",
    "deletes", "deny", "denied", "denies", "didn", "disable", "disabled", "disables", "doesn", "don",
    "enable", "enabled", "enables", "fail", "failed", "failing", "fails", "hadn", "hasn",
    "haven", "if", "ignore", "ignored", "ignores", "isn", "may", "might", "must", "never",
    "no", "not", "pass", "passed", "passes", "passing", "plan", "planned", "planning", "plans",
    "prevent", "prevented", "prevents", "reject", "rejected", "rejects",
    "remove", "removed", "removes", "shall", "should", "shouldn", "skip", "skipped", "skips",
    "succeed", "succeeded", "succeeds", "success", "unless", "wasn", "weren", "will", "without",
    "won", "wouldn",
];

pub(super) fn has_conservative_source_support(
    candidate_text: &str,
    observation_text: &str,
) -> bool {
    if contains_support_risk_token(candidate_text) {
        return false;
    }
    has_conservative_exact_support(candidate_text, observation_text)
        || has_conservative_support_token_overlap(candidate_text, observation_text)
}

fn has_conservative_exact_support(candidate_text: &str, observation_text: &str) -> bool {
    support_sentence_segments(observation_text)
        .into_iter()
        .any(|segment| !contains_support_risk_token(&segment) && segment.contains(candidate_text))
}

fn has_conservative_support_token_overlap(candidate_text: &str, observation_text: &str) -> bool {
    let candidate_tokens = support_tokens(candidate_text);
    if candidate_tokens.len() < MIN_SUPPORT_TOKEN_OVERLAP {
        return false;
    }
    let candidate_required = candidate_tokens
        .iter()
        .filter(|token| token.required)
        .count();
    support_text_segments(observation_text)
        .into_iter()
        .any(|segment| {
            !contains_support_risk_token(&segment)
                && has_conservative_support_token_overlap_segment(
                    &candidate_tokens,
                    &segment,
                    candidate_required,
                )
        })
}

fn has_conservative_support_token_overlap_segment(
    candidate_tokens: &[SupportToken],
    observation_text: &str,
    candidate_required: usize,
) -> bool {
    let observation_tokens = support_tokens(observation_text);
    has_ordered_support_window(candidate_tokens, &observation_tokens, candidate_required)
}

fn has_ordered_support_window(
    candidate_tokens: &[SupportToken],
    observation_tokens: &[SupportToken],
    candidate_required: usize,
) -> bool {
    let Some(first_candidate) = candidate_tokens.first() else {
        return false;
    };
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
            && matched >= MIN_SUPPORT_TOKEN_OVERLAP
            && required_matched == candidate_required
            && (matched as f64 / candidate_tokens.len() as f64) >= MIN_SUPPORT_TOKEN_RATIO
        {
            return true;
        }
    }
    false
}

fn support_tokens(text: &str) -> Vec<SupportToken> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(support_token)
        .collect()
}

fn support_sentence_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut segment_start = 0;
    for (index, ch) in text.char_indices() {
        if is_support_sentence_boundary_char(ch) {
            push_support_text_segment(text, segment_start, index + ch.len_utf8(), &mut segments);
            segment_start = index + ch.len_utf8();
        }
    }
    push_support_text_segment(text, segment_start, text.len(), &mut segments);
    segments
}

fn support_text_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut segment_start = 0;
    let mut token_start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_ascii_alphanumeric() {
            if token_start.is_none() {
                token_start = Some(index);
            }
        } else {
            if let Some(start) = token_start.take() {
                let token = text[start..index].to_ascii_lowercase();
                if is_support_clause_boundary_token(&token) {
                    push_support_text_segment(text, segment_start, start, &mut segments);
                    segment_start = index + ch.len_utf8();
                }
            }
            if is_support_clause_boundary_char(ch) {
                push_support_text_segment(text, segment_start, index, &mut segments);
                segment_start = index + ch.len_utf8();
            }
        }
    }
    if let Some(start) = token_start {
        let token = text[start..].to_ascii_lowercase();
        if is_support_clause_boundary_token(&token) {
            push_support_text_segment(text, segment_start, start, &mut segments);
            segment_start = text.len();
        }
    }
    push_support_text_segment(text, segment_start, text.len(), &mut segments);
    segments
}

fn push_support_text_segment(text: &str, start: usize, end: usize, segments: &mut Vec<String>) {
    if start >= end {
        return;
    }
    let segment = text[start..end].trim();
    if !segment.is_empty() {
        segments.push(segment.to_string());
    }
}

fn is_support_clause_boundary_char(ch: char) -> bool {
    matches!(ch, '.' | ';' | ':' | '?' | '!')
}

fn is_support_sentence_boundary_char(ch: char) -> bool {
    matches!(ch, '.' | ';' | '?' | '!')
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
        "aes"
            | "api"
            | "cli"
            | "db"
            | "jwt"
            | "kms"
            | "llm"
            | "mcp"
            | "rsa"
            | "s3"
            | "sql"
            | "ssh"
            | "ssl"
            | "tls"
            | "ui"
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
