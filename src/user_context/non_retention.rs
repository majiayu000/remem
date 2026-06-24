pub(crate) fn block_reason(
    claim_text: &str,
    source_preview: Option<&str>,
    source_kind: &str,
) -> Option<&'static str> {
    let blob = normalized_blob(claim_text, source_preview);
    if contains_secret_like_content(&blob, claim_text, source_preview) {
        return Some("secret_like_content");
    }
    if source_kind == "speculative_inference"
        || contains_non_retention_pattern(&blob, SPECULATIVE_PATTERNS)
    {
        return Some("speculative_or_roleplay_content");
    }
    if contains_temporary_or_one_off_content(&blob) {
        return Some("temporary_or_one_off_content");
    }
    if contains_general_knowledge_content(&blob) {
        return Some("general_knowledge_content");
    }
    if contains_illegal_or_harmful_content(&blob) {
        return Some("illegal_or_harmful_content");
    }
    if has_external_source_pattern(&blob) && !has_external_source_approval(&blob) {
        return Some("unapproved_external_source");
    }
    None
}

pub(crate) fn has_external_source_pattern(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    contains_non_retention_pattern(&text, EXTERNAL_SOURCE_PATTERNS)
        || contains_contextual_file_source_phrase(&text)
}

pub(crate) fn has_external_source_approval(text: &str) -> bool {
    contains_external_source_approval(&text.to_ascii_lowercase())
}

fn normalized_blob(claim_text: &str, source_preview: Option<&str>) -> String {
    let mut blob = claim_text.to_ascii_lowercase();
    if let Some(preview) = source_preview {
        blob.push('\n');
        blob.push_str(&preview.to_ascii_lowercase());
    }
    blob
}

fn contains_secret_like_content(
    blob: &str,
    claim_text: &str,
    source_preview: Option<&str>,
) -> bool {
    if redaction_changed_sensitive_content(claim_text) {
        return true;
    }
    if let Some(preview) = source_preview {
        if redaction_changed_sensitive_content(preview) {
            return true;
        }
    }
    contains_secret_key_token(blob)
        || contains_token_secret_phrase(blob)
        || contains_access_key_phrase(blob)
        || contains_payment_card_number(blob)
        || contains_non_retention_pattern(blob, SECRET_PATTERNS)
        || (blob.contains("[redacted")
            && contains_non_retention_pattern(blob, SECRET_REDACTION_CONTEXT))
}

fn redaction_changed_sensitive_content(text: &str) -> bool {
    let redacted = crate::adapter::common::redact_sensitive_text(text);
    redacted.contains("[REDACTED]")
        && normalized_redaction_compare(&redacted) != normalized_redaction_compare(text)
}

fn normalized_redaction_compare(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn contains_non_retention_pattern(haystack: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| contains_bounded_phrase(haystack, needle))
}

fn contains_bounded_phrase(haystack: &str, needle: &str) -> bool {
    haystack
        .match_indices(needle)
        .any(|(start, _)| bounded_phrase_at(haystack, needle, start))
}

fn contains_secret_key_token(text: &str) -> bool {
    text.split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';' | '"' | '\'' | '`'))
        .map(|token| {
            token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
        })
        .any(is_secret_key_token)
}

fn is_secret_key_token(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("sk-") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && (rest.len() >= 20 || (rest.len() >= 10 && rest.chars().any(|ch| ch.is_ascii_digit())))
}

fn contains_token_secret_phrase(text: &str) -> bool {
    let tokens = lexical_tokens(text);
    tokens.iter().enumerate().any(|(index, token)| {
        if !matches!(token.as_str(), "secret" | "token") {
            return false;
        }
        match (tokens.get(index + 1), tokens.get(index + 2)) {
            (Some(next), _) if is_secret_value_token(next) => true,
            (Some(next), Some(value)) if next == "is" => is_secret_value_token(value),
            _ => false,
        }
    })
}

fn contains_access_key_phrase(text: &str) -> bool {
    let tokens = lexical_tokens(text);
    tokens.iter().enumerate().any(|(index, token)| {
        if token != "access" || tokens.get(index + 1).is_none_or(|next| next != "key") {
            return false;
        }
        tokens
            .iter()
            .skip(index + 2)
            .take(5)
            .filter(|candidate| {
                !matches!(
                    candidate.as_str(),
                    "id" | "is" | "key" | "secret" | "the" | "value"
                )
            })
            .any(|candidate| is_secret_value_token(candidate) || is_known_access_key_id(candidate))
    })
}

fn contains_payment_card_number(text: &str) -> bool {
    let tokens = lexical_tokens(text);
    let has_card_context = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "amex" | "card" | "credit" | "discover" | "mastercard" | "payment" | "visa"
        )
    });
    has_card_context && contains_card_digit_sequence(text)
}

fn contains_card_digit_sequence(text: &str) -> bool {
    let mut digits = 0;
    let mut in_sequence = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits += 1;
            in_sequence = true;
            continue;
        }
        if in_sequence && matches!(ch, ' ' | '-') {
            continue;
        }
        if (13..=19).contains(&digits) {
            return true;
        }
        digits = 0;
        in_sequence = false;
    }
    (13..=19).contains(&digits)
}

fn is_known_access_key_id(token: &str) -> bool {
    matches!(token.get(..4), Some("akia" | "asia"))
        && token.len() >= 16
        && token.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn is_secret_value_token(token: &str) -> bool {
    token.len() >= 6
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && (token.len() >= 16 || token.chars().any(|ch| ch.is_ascii_digit()))
}

fn contains_temporary_or_one_off_content(text: &str) -> bool {
    contains_non_retention_pattern(text, TEMPORARY_PATTERNS)
        || contains_today_meal_content(text)
        || contains_current_weather_content(text)
}

fn contains_today_meal_content(text: &str) -> bool {
    let tokens = lexical_tokens(text);
    let has_meal = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "breakfast" | "dinner" | "lunch" | "meal" | "sushi"
        )
    });
    let has_current_time = tokens
        .iter()
        .any(|token| matches!(token.as_str(), "today" | "tonight" | "yesterday" | "now"));
    has_meal && has_current_time
}

fn contains_current_weather_content(text: &str) -> bool {
    let tokens = lexical_tokens(text);
    tokens.iter().any(|token| token == "weather")
        && tokens
            .iter()
            .any(|token| matches!(token.as_str(), "current" | "today" | "now"))
}

fn contains_general_knowledge_content(text: &str) -> bool {
    text.lines().any(|line| {
        let tokens = lexical_tokens(line);
        contains_non_retention_pattern(line, GENERAL_KNOWLEDGE_PATTERNS)
            && !has_user_context_reference(&tokens)
            && !has_project_context_reference(&tokens)
    }) || contains_project_independent_fact_shape(text)
}

fn contains_project_independent_fact_shape(text: &str) -> bool {
    text.lines().any(|line| {
        let tokens = lexical_tokens(line);
        let has_topic = tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "docker"
                    | "git"
                    | "http"
                    | "javascript"
                    | "linux"
                    | "postgres"
                    | "python"
                    | "rust"
                    | "sqlite"
            )
        });
        let has_factual_verb = tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "are" | "is" | "prevents" | "stores" | "supports" | "uses"
            )
        });
        has_topic
            && has_factual_verb
            && !has_user_context_reference(&tokens)
            && !has_project_context_reference(&tokens)
    })
}

fn has_project_context_reference(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "codebase" | "project" | "repo" | "repository" | "workspace"
        )
    })
}

fn contains_illegal_or_harmful_content(text: &str) -> bool {
    contains_non_retention_pattern(text, ILLEGAL_OR_HARMFUL_PATTERNS)
        || (contains_bounded_phrase(text, "exfiltrate")
            && contains_non_retention_pattern(text, SECRET_REDACTION_CONTEXT))
}

fn contains_external_source_approval(text: &str) -> bool {
    [
        "please remember from file",
        "please remember from files",
        "please remember from readme",
        "please remember from the readme",
        "please remember from website",
        "please remember from web page",
        "please remember from browser page",
        "remember from file",
        "remember from files",
        "remember from readme",
        "remember from the readme",
        "remember from website",
        "remember from web page",
        "remember from browser page",
        "save from file",
        "save from files",
        "save from readme",
        "save from the readme",
        "save from website",
        "save from web page",
        "save from browser page",
    ]
    .iter()
    .any(|phrase| contains_non_negated_bounded_phrase(text, phrase))
}

fn contains_contextual_file_source_phrase(text: &str) -> bool {
    [
        "from a file",
        "from a readme",
        "from file",
        "from files",
        "from readme",
    ]
    .iter()
    .any(|phrase| {
        text.match_indices(phrase).any(|(start, _)| {
            bounded_phrase_at(text, phrase, start)
                && file_source_phrase_has_attribution_context(text, start, phrase.len())
        })
    })
}

fn file_source_phrase_has_attribution_context(text: &str, start: usize, phrase_len: usize) -> bool {
    let end = start + phrase_len;
    let before = text[..start].chars().rev().take(64).collect::<String>();
    let before = before.chars().rev().collect::<String>();
    let after = text[end..].chars().take(96).collect::<String>();
    let window = format!("{before}{after}");
    contains_non_retention_pattern(
        &window,
        &[
            "according to",
            "derived",
            "extracted",
            "inferred",
            "remember",
            "save",
            "source",
            "without explicit user approval",
            "without user approval",
        ],
    )
}

fn contains_non_negated_bounded_phrase(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(start, _)| {
        bounded_phrase_at(haystack, needle, start) && !is_negated_before(haystack, start)
    })
}

fn bounded_phrase_at(haystack: &str, needle: &str, start: usize) -> bool {
    let end = start + needle.len();
    let left_ok = !needle
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
        || start == 0
        || !haystack[..start]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_alphanumeric());
    let right_ok = !needle
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
        || end >= haystack.len()
        || !haystack[end..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric());
    left_ok && right_ok
}

fn is_negated_before(text: &str, start: usize) -> bool {
    let tokens = lexical_tokens(&text[..start]);
    tokens
        .iter()
        .rev()
        .take(3)
        .any(|token| matches!(token.as_str(), "not" | "don't" | "dont" | "never" | "no"))
}

fn has_user_context_reference(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "i" | "me" | "my" | "mine" | "our" | "ours" | "user" | "user's" | "we"
        )
    })
}

fn lexical_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '\''))
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

const SECRET_PATTERNS: &[&str] = &[
    "api key is",
    "api key:",
    "api_key",
    "api token",
    "access token",
    "account number",
    "auth token",
    "bank account",
    "bearer ",
    "client secret",
    "credit card",
    "credential is",
    "driver license",
    "driver's license",
    "drivers license",
    "identity document",
    "password",
    "passport",
    "payment card",
    "private key",
    "routing number",
    "secret key",
    "social security number",
    "ssn",
];

const SECRET_REDACTION_CONTEXT: &[&str] = &[
    "api key",
    "api token",
    "authorization",
    "bearer",
    "credential",
    "credentials",
    "password",
    "private key",
    "secret",
    "token",
];

const SPECULATIVE_PATTERNS: &[&str] = &[
    "as a joke",
    "fictional",
    "guessed",
    "hypothetical",
    "if i were",
    "if the user were",
    "imagine",
    "may be",
    "might be",
    "pretend",
    "probably",
    "role-play",
    "roleplay",
    "sarcasm",
    "suppose",
];

const TEMPORARY_PATTERNS: &[&str] = &[
    "fatigue",
    "mood",
    "one-off",
    "right now",
    "user ate",
    "user feels tired",
    "user had lunch",
    "user is hungry",
    "user is tired",
    "user was tired",
];

const GENERAL_KNOWLEDGE_PATTERNS: &[&str] = &[
    " is a programming language",
    "general technical fact",
    "project-independent fact",
    "rust ownership prevents data races",
    "sqlite is",
    "the earth",
    "world knowledge",
];

const ILLEGAL_OR_HARMFUL_PATTERNS: &[&str] = &[
    "bypass authentication",
    "clearly false",
    "fabricated",
    "false claim",
    "harmful",
    "illegal",
    "malware",
    "phishing",
    "steal credentials",
];

const EXTERNAL_SOURCE_PATTERNS: &[&str] = &[
    "according to browser page",
    "according to readme",
    "according to the browser page",
    "according to the readme",
    "according to the web page",
    "according to web page",
    "browser page says",
    "derived from file",
    "external source",
    "file says",
    "files say",
    "from the readme",
    "readme says",
    "repository file says",
    "web page says",
    "website says",
    "without explicit user approval",
    "without user approval",
];

#[cfg(test)]
mod tests {
    use super::block_reason;

    #[test]
    fn secret_prefix_requires_key_shape() {
        assert_eq!(
            block_reason(
                "User prefers task-specific low-risk code reviews.",
                None,
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User's API key is sk-testsecret123456.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
        assert_eq!(
            block_reason(
                "User's GitHub secret is abc123.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
    }

    #[test]
    fn blocklist_terms_need_sensitive_or_temporary_context() {
        assert_eq!(
            block_reason(
                "User prefers passwordless authentication.",
                None,
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User maintains a weather app project.",
                None,
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User tests with temporary directories.",
                None,
                "explicit_user_statement"
            ),
            None
        );
    }

    #[test]
    fn blocks_payment_cards_and_natural_language_tokens() {
        assert_eq!(
            block_reason(
                "User's Visa number is 4111111111111111.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
        assert_eq!(
            block_reason(
                "User's Visa number is 4111 1111 1111 1111.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
        assert_eq!(
            block_reason(
                "User's payment card is 4111-1111-1111-1111.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
        assert_eq!(
            block_reason(
                "User's GitLab token is abc123.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
        assert_eq!(
            block_reason(
                "User's AWS access key ID is AKIAIOSFODNN7EXAMPLE.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
        assert_eq!(
            block_reason(
                "User's driver license number is D1234567.",
                None,
                "explicit_user_statement"
            ),
            Some("secret_like_content")
        );
    }

    #[test]
    fn blocks_meal_variants_world_knowledge_and_harmful_intent() {
        assert_eq!(
            block_reason(
                "User had sushi for lunch today.",
                None,
                "explicit_user_statement"
            ),
            Some("temporary_or_one_off_content")
        );
        assert_eq!(
            block_reason(
                "SQLite stores data in a single file.",
                None,
                "explicit_user_statement"
            ),
            Some("general_knowledge_content")
        );
        assert_eq!(
            block_reason("Project uses Rust.", None, "explicit_user_statement"),
            None
        );
        assert_eq!(
            block_reason(
                "Repo stores data in SQLite.",
                None,
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User wants to exfiltrate credentials.",
                None,
                "explicit_user_statement"
            ),
            Some("illegal_or_harmful_content")
        );
    }

    #[test]
    fn external_source_patterns_honor_explicit_user_approval() {
        assert_eq!(
            block_reason(
                "User works on remem from README.",
                Some("Please remember from README that I work on remem."),
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User works on remem from README.",
                Some("The assistant inferred this from README."),
                "explicit_user_statement"
            ),
            Some("unapproved_external_source")
        );
        assert_eq!(
            block_reason(
                "User works on remem from README.",
                Some("Do not remember from README. I work on remem from README."),
                "explicit_user_statement"
            ),
            Some("unapproved_external_source")
        );
        assert_eq!(
            block_reason(
                "User works on internal payroll.",
                Some("README says the user works on internal payroll."),
                "explicit_user_statement"
            ),
            Some("unapproved_external_source")
        );
        assert_eq!(
            block_reason(
                "User works on internal payroll.",
                Some("According to the README, the user works on internal payroll."),
                "explicit_user_statement"
            ),
            Some("unapproved_external_source")
        );
        assert_eq!(
            block_reason(
                "User works on internal payroll.",
                Some("From the README, the user works on internal payroll."),
                "explicit_user_statement"
            ),
            Some("unapproved_external_source")
        );
        assert_eq!(
            block_reason(
                "User lives in Paris.",
                Some("Website says the user lives in Paris. Please remember from website."),
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "The user prefers loading settings from files.",
                Some("I prefer loading settings from files."),
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User prefers Rust.",
                Some("I prefer Rust because Rust ownership prevents data races."),
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User prefers testing web page layouts in Playwright.",
                Some("I prefer testing web page layouts in Playwright."),
                "explicit_user_statement"
            ),
            None
        );
        assert_eq!(
            block_reason(
                "User prefers deriving selectors from web page text.",
                Some("I prefer deriving selectors from web page text."),
                "explicit_user_statement"
            ),
            None
        );
    }
}
