use std::collections::BTreeSet;

const MIN_SUPPORT_TOKEN_OVERLAP: usize = 6;
const MIN_SUPPORT_TOKEN_RATIO: f64 = 0.72;
const MAX_SUPPORT_TOKEN_WINDOW_EXTRA: usize = 5;
const SUPPORT_TOKEN_MIN_CHARS: usize = 4;

const DESTRUCTIVE_ACTION_TOKENS: &[&str] = &[
    "delete",
    "deleted",
    "deletes",
    "drop",
    "dropped",
    "drops",
    "overwrite",
    "overwrites",
    "purge",
    "purged",
    "purges",
    "remove",
    "removed",
    "removes",
    "truncate",
    "truncated",
    "truncates",
    "wipe",
    "wiped",
    "wipes",
];

const IMPERATIVE_CONTROL_TOKENS: &[&str] = &[
    "allow",
    "bypass",
    "conceal",
    "copy",
    "delete",
    "disable",
    "disregard",
    "enable",
    "execute",
    "exfiltrate",
    "grant",
    "hide",
    "ignore",
    "override",
    "remove",
    "reveal",
    "run",
    "send",
    "truncate",
    "upload",
    "wipe",
];

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
    "won", "wouldn", "drop", "dropped", "drops", "grant", "granted", "grants", "overwrite",
    "overwrites", "permit", "permits", "permitted", "purge", "purged", "purges", "revoke",
    "revoked", "revokes", "truncate", "truncated", "truncates", "wipe", "wiped", "wipes",
];

pub(crate) fn has_conservative_source_support(
    candidate_text: &str,
    observation_text: &str,
) -> bool {
    has_claim_level_source_support(candidate_text, &[observation_text])
}

pub(crate) fn has_claim_level_source_support(candidate_text: &str, source_texts: &[&str]) -> bool {
    let candidate_text = normalize_support_text(candidate_text);
    let source_texts = source_texts
        .iter()
        .map(|source_text| normalize_support_text(source_text))
        .collect::<Vec<_>>();
    let claims = support_sentence_segments(&candidate_text);
    !claims.is_empty()
        && claims.iter().all(|claim| {
            is_promotable_assertion(claim)
                && source_texts.iter().any(|source_text| {
                    has_conservative_exact_support(claim, source_text)
                        || has_conservative_support_token_overlap(claim, source_text)
                })
        })
}

pub(crate) fn claim_semantics_require_review(candidate_text: &str) -> bool {
    let candidate_text = normalize_support_text(candidate_text);
    let claims = support_sentence_segments(&candidate_text);
    claims.is_empty() || claims.iter().any(|claim| !is_promotable_assertion(claim))
}

fn normalize_support_text(text: &str) -> String {
    fold_width_and_case(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_conservative_exact_support(candidate_text: &str, observation_text: &str) -> bool {
    support_sentence_segments(observation_text)
        .into_iter()
        .any(|segment| {
            semantic_signatures_match(candidate_text, &segment) && segment.contains(candidate_text)
        })
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
            semantic_signatures_match(candidate_text, &segment)
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
    normalize_contractions(text)
        .split(|ch: char| !ch.is_ascii_alphanumeric())
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
    let required_semantic = SUPPORT_RISK_TOKENS.contains(&token);
    if !required_identifier && !required_semantic && token.chars().count() < SUPPORT_TOKEN_MIN_CHARS
    {
        return None;
    }
    let text = normalize_support_token(token);
    Some(SupportToken {
        required: required_identifier || required_semantic || !is_optional_support_token(&text),
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

fn normalize_contractions(text: &str) -> String {
    fold_width_and_case(text)
        .replace(['’', '‘', 'ʼ', '＇'], "'")
        .replace("won't", "will not")
        .replace("can't", "can not")
        .replace("shan't", "shall not")
        .replace("n't", " not")
}

fn is_promotable_assertion(text: &str) -> bool {
    let signature = semantic_signature(text);
    signature.iter().all(|semantic| {
        !matches!(
            *semantic,
            "conditional"
                | "imperative_control"
                | "meta_negated"
                | "modal_capability"
                | "prescriptive"
                | "prospective"
                | "security_sensitive"
                | "uncertain"
        )
    }) && destructive_actions_are_safely_negated(text)
}

fn semantic_signatures_match(candidate_text: &str, source_text: &str) -> bool {
    semantic_signature(candidate_text) == semantic_signature(source_text)
}

fn semantic_signature(text: &str) -> BTreeSet<&'static str> {
    let normalized = normalize_contractions(text);
    let tokens = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut signature = tokens
        .iter()
        .filter_map(|token| match *token {
            "no" | "not" | "never" | "cannot" | "cant" | "couldn" | "didn" | "doesn" | "don"
            | "hadn" | "hasn" | "haven" | "isn" | "shouldn" | "wasn" | "weren" | "won"
            | "wouldn" => Some("negative"),
            "may" | "might" | "could" | "would" => Some("uncertain"),
            "will" | "plan" | "planned" | "planning" | "plans" => Some("prospective"),
            "can" => Some("modal_capability"),
            "must" | "shall" | "should" => Some("prescriptive"),
            "if" | "unless" => Some("conditional"),
            "fail" | "failed" | "failing" | "fails" | "failure" | "failures" => {
                Some("outcome_fail")
            }
            "pass" | "passed" | "passes" | "passing" | "succeed" | "succeeded" | "succeeds"
            | "success" => Some("outcome_success"),
            "allow" | "allowed" | "allows" => Some("control_allow"),
            "grant" | "granted" | "grants" | "permit" | "permits" | "permitted" => {
                Some("control_allow")
            }
            "enable" | "enabled" | "enables" => Some("control_enable"),
            "deny" | "denied" | "denies" => Some("control_deny"),
            "revoke" | "revoked" | "revokes" => Some("control_deny"),
            "disable" | "disabled" | "disables" => Some("control_disable"),
            "reject" | "rejected" | "rejects" => Some("control_reject"),
            "prevent" | "prevented" | "prevents" => Some("control_prevent"),
            "ignore" | "ignored" | "ignores" => Some("control_ignore"),
            "skip" | "skipped" | "skips" => Some("control_skip"),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if tokens
        .iter()
        .any(|token| matches!(*token, "cannot" | "cant"))
    {
        signature.insert("modal_capability");
    }
    if tokens
        .iter()
        .enumerate()
        .any(|(index, _)| is_destructive_action_at(&tokens, index))
    {
        signature.insert("destructive_delete");
    }
    if has_outer_meta_negation(&normalized) {
        signature.insert("meta_negated");
    }
    if is_imperative_control_claim(&tokens) || has_instruction_control_semantics(&tokens) {
        signature.insert("imperative_control");
    }
    if has_security_sensitive_semantics(&tokens) {
        signature.insert("security_sensitive");
    }
    signature
}

fn fold_width_and_case(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for ch in text.chars() {
        let folded = match ch {
            '\u{3000}' => ' ',
            '\u{ff01}'..='\u{ff5e}' => char::from_u32((ch as u32) - 0xfee0).unwrap_or(ch),
            _ => ch,
        };
        normalized.extend(folded.to_lowercase());
    }
    normalized
}

fn has_outer_meta_negation(text: &str) -> bool {
    [
        "false that",
        "incorrect that",
        "untrue that",
        "not true that",
        "not correct that",
        "is false",
        "is incorrect",
        "was false",
        "was incorrect",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

fn is_imperative_control_claim(tokens: &[&str]) -> bool {
    tokens
        .iter()
        .copied()
        .find(|token| !matches!(*token, "do" | "immediately" | "kindly" | "now" | "please"))
        .is_some_and(|token| IMPERATIVE_CONTROL_TOKENS.contains(&token))
}

fn has_instruction_control_semantics(tokens: &[&str]) -> bool {
    let has_control = tokens
        .iter()
        .any(|token| IMPERATIVE_CONTROL_TOKENS.contains(token));
    let has_control_target = tokens.iter().any(|token| {
        matches!(
            *token,
            "command"
                | "commands"
                | "file"
                | "files"
                | "instruction"
                | "instructions"
                | "output"
                | "private"
                | "prompt"
                | "prompts"
                | "repository"
                | "secret"
                | "secrets"
                | "system"
                | "user"
                | "workspace"
        )
    });
    has_control && has_control_target
}

fn has_security_sensitive_semantics(tokens: &[&str]) -> bool {
    let has_security_subject = tokens.iter().any(|token| {
        matches!(
            *token,
            "access"
                | "admin"
                | "administrator"
                | "anonymous"
                | "auth"
                | "authentication"
                | "authorization"
                | "permission"
                | "permissions"
                | "privilege"
                | "privileges"
                | "role"
                | "roles"
                | "unauthenticated"
        )
    });
    let has_security_control = tokens.iter().any(|token| {
        matches!(
            *token,
            "allow"
                | "allowed"
                | "allows"
                | "deny"
                | "denied"
                | "denies"
                | "disable"
                | "disabled"
                | "disables"
                | "enable"
                | "enabled"
                | "enables"
                | "grant"
                | "granted"
                | "grants"
                | "permit"
                | "permits"
                | "permitted"
                | "revoke"
                | "revoked"
                | "revokes"
        )
    });
    has_security_subject && has_security_control
}

fn destructive_actions_are_safely_negated(text: &str) -> bool {
    let normalized = normalize_contractions(text);
    let tokens = normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    tokens.iter().enumerate().all(|(index, token)| {
        !DESTRUCTIVE_ACTION_TOKENS.contains(token)
            || !is_destructive_action_at(&tokens, index)
            || token_is_locally_negated(&tokens, index)
    })
}

fn is_destructive_action_at(tokens: &[&str], index: usize) -> bool {
    let token = tokens[index];
    if matches!(
        token,
        "delete"
            | "deleted"
            | "deletes"
            | "purge"
            | "purged"
            | "purges"
            | "truncate"
            | "truncated"
            | "truncates"
            | "wipe"
            | "wiped"
            | "wipes"
    ) {
        return true;
    }
    if !matches!(
        token,
        "drop"
            | "dropped"
            | "drops"
            | "overwrite"
            | "overwrites"
            | "remove"
            | "removed"
            | "removes"
    ) {
        return false;
    }
    tokens[index + 1..tokens.len().min(index + 5)]
        .iter()
        .take_while(|token| !matches!(**token, "and" | "but" | "however" | "then"))
        .any(|token| {
            matches!(
                *token,
                "archive"
                    | "archives"
                    | "database"
                    | "databases"
                    | "file"
                    | "files"
                    | "index"
                    | "indexes"
                    | "indices"
                    | "memory"
                    | "memories"
                    | "row"
                    | "rows"
                    | "schema"
                    | "schemas"
                    | "table"
                    | "tables"
            )
        })
}

fn token_is_locally_negated(tokens: &[&str], index: usize) -> bool {
    let start = index.saturating_sub(3);
    for token in tokens[start..index].iter().rev() {
        if matches!(*token, "and" | "but" | "however" | "then") {
            return false;
        }
        if matches!(
            *token,
            "cannot" | "cant" | "never" | "no" | "not" | "without"
        ) {
            return true;
        }
    }
    false
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
