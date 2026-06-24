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
    if contains_non_retention_pattern(&blob, TEMPORARY_PATTERNS) {
        return Some("temporary_or_one_off_content");
    }
    if contains_non_retention_pattern(&blob, GENERAL_KNOWLEDGE_PATTERNS) {
        return Some("general_knowledge_content");
    }
    if contains_non_retention_pattern(&blob, ILLEGAL_OR_HARMFUL_PATTERNS) {
        return Some("illegal_or_harmful_content");
    }
    if contains_non_retention_pattern(&blob, EXTERNAL_SOURCE_PATTERNS) {
        return Some("unapproved_external_source");
    }
    None
}

pub(crate) fn unsupported_assistant_claim_reason(
    _source_kind: &str,
    has_user_authored_source: bool,
) -> Option<&'static str> {
    if has_user_authored_source {
        return None;
    }
    Some("unsupported_assistant_authored_claim")
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
    if crate::adapter::common::redact_sensitive_text(claim_text) != claim_text {
        return true;
    }
    if let Some(preview) = source_preview {
        if crate::adapter::common::redact_sensitive_text(preview) != preview {
            return true;
        }
    }
    contains_non_retention_pattern(blob, SECRET_PATTERNS)
        || (blob.contains("[redacted")
            && contains_non_retention_pattern(blob, SECRET_REDACTION_CONTEXT))
}

fn contains_non_retention_pattern(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

const SECRET_PATTERNS: &[&str] = &[
    "api key is",
    "api key:",
    "api_key",
    "api token",
    "access token",
    "auth token",
    "bearer ",
    "client secret",
    "credit card",
    "credential is",
    "driver's license",
    "drivers license",
    "identity document",
    "password",
    "passport",
    "payment card",
    "private key",
    "secret key",
    "ssn",
    "sk-",
];

const SECRET_REDACTION_CONTEXT: &[&str] = &[
    "api key",
    "api token",
    "authorization",
    "bearer",
    "credential",
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
    "temporary",
    "user ate",
    "user feels tired",
    "user had lunch",
    "user is hungry",
    "user is tired",
    "user was tired",
    "weather",
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
    "browser page",
    "derived from file",
    "external source",
    "from a file",
    "from a readme",
    "from file",
    "from files",
    "from readme",
    "repository file says",
    "web page",
    "website says",
    "without explicit user approval",
    "without user approval",
];
