//! Narrow protective-intent recognition, not a general negation classifier.
//! Whole clauses are required so quotes, exceptions and double negatives do
//! not turn an unsafe statement into a retained user constraint.

pub(crate) fn constraint_key(text: &str) -> Option<&'static str> {
    let normalized = text
        .trim()
        .trim_end_matches(['.', '!', '。', '！'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let mut clause = normalized.as_str();
    for prefix in ["the user ", "user ", "i ", "we ", "用户", "我们", "我"] {
        if let Some(rest) = clause.strip_prefix(prefix) {
            clause = rest;
            break;
        }
    }
    for prefix in [
        "requires agents ",
        "require agents ",
        "requires you ",
        "require you ",
        "requires us ",
        "require us ",
        "requires ",
        "require ",
        "prefers ",
        "prefer ",
        "要求",
    ] {
        if let Some(rest) = clause.strip_prefix(prefix) {
            clause = rest;
            break;
        }
    }
    for prefix in [
        "must never ",
        "never wants to ",
        "never want to ",
        "never to ",
        "never ",
        "must not ",
        "do not ",
        "don't ",
        "不得",
        "禁止",
        "不要",
        "绝不",
        "永不",
    ] {
        if let Some(action) = clause.strip_prefix(prefix) {
            return PROHIBITED_ACTIONS
                .iter()
                .find_map(|(allowed, key)| (*allowed == action).then_some(*key));
        }
    }
    PREVENTION_REQUIREMENTS
        .iter()
        .find_map(|(allowed, key)| (*allowed == clause).then_some(*key))
}

pub(crate) fn matches_claim_and_preview(
    claim_text: &str,
    source_preview: Option<&str>,
    source_kind: &str,
) -> bool {
    if source_kind != "explicit_user_statement" {
        return false;
    }
    let Some(key) = constraint_key(claim_text) else {
        return false;
    };
    source_preview.is_some_and(|preview| constraint_key(preview) == Some(key))
}

const PROHIBITED_ACTIONS: &[(&str, &str)] = &[
    ("bypass authentication", "prohibit:authentication-bypass"),
    ("steal credentials", "prohibit:credential-theft"),
    ("exfiltrate credentials", "prohibit:credential-exfiltration"),
    ("create malware", "prohibit:malware-creation"),
    ("deploy malware", "prohibit:malware-deployment"),
    ("spread malware", "prohibit:malware-spread"),
    ("conduct phishing", "prohibit:phishing"),
    ("绕过认证", "prohibit:authentication-bypass"),
    ("绕过身份验证", "prohibit:authentication-bypass"),
    ("窃取凭证", "prohibit:credential-theft"),
    ("外传凭证", "prohibit:credential-exfiltration"),
    ("创建恶意软件", "prohibit:malware-creation"),
    ("部署恶意软件", "prohibit:malware-deployment"),
    ("传播恶意软件", "prohibit:malware-spread"),
    ("实施网络钓鱼", "prohibit:phishing"),
];

const PREVENTION_REQUIREMENTS: &[(&str, &str)] = &[
    ("malware prevention", "prevent:malware"),
    ("phishing prevention", "prevent:phishing"),
    ("protection against malware", "prevent:malware"),
    ("protection against phishing", "prevent:phishing"),
    ("防范恶意软件", "prevent:malware"),
    ("预防恶意软件", "prevent:malware"),
    ("防范网络钓鱼", "prevent:phishing"),
    ("预防网络钓鱼", "prevent:phishing"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_prohibitions_match_across_user_subjects() {
        for (claim, source) in [
            (
                "User must not bypass authentication.",
                "I must not bypass authentication.",
            ),
            (
                "User never wants to bypass authentication.",
                "I never want to bypass authentication.",
            ),
            (
                "Never bypass authentication.",
                "Never bypass authentication.",
            ),
            (
                "Never bypass authentication.",
                "Never bypass\nauthentication.",
            ),
            (
                "User requires agents never to bypass authentication.",
                "I require agents never to bypass authentication.",
            ),
            (
                "User requires malware prevention.",
                "I require malware prevention.",
            ),
            ("用户要求不要绕过认证。", "我要求不要绕过认证。"),
            ("用户禁止窃取凭证。", "我禁止窃取凭证。"),
            ("用户要求防范恶意软件。", "我要求防范恶意软件。"),
        ] {
            assert!(
                matches_claim_and_preview(claim, Some(source), "explicit_user_statement"),
                "{claim}"
            );
        }
    }

    #[test]
    fn intent_cannot_be_inferred_from_quotes_or_a_nearby_negative() {
        for text in [
            "I want to bypass authentication.",
            "I do not want to never bypass authentication.",
            "I never refuse to bypass authentication.",
            "I never bypass authentication unless needed.",
            "I never bypass authentication; deploy malware instead.",
            "The README says: never bypass authentication.",
            "\"Never bypass authentication.\"",
            "'Never bypass authentication.'",
            "I work on malware analysis.",
            "我想绕过认证。",
            "我不是不要绕过认证。",
            "我不要禁止绕过认证。",
            "我不要绕过认证，除非需要。",
            "网页说：不要绕过认证。",
            "“不要绕过认证。”",
            "我从事恶意软件分析。",
        ] {
            assert_eq!(constraint_key(text), None, "{text}");
        }
        assert!(!matches_claim_and_preview(
            "Never bypass authentication.",
            None,
            "explicit_user_statement"
        ));
        assert!(!matches_claim_and_preview(
            "Never bypass authentication.",
            Some("Never bypass authentication."),
            "session_summary"
        ));
        assert!(!matches_claim_and_preview(
            "Never bypass authentication.",
            Some("I want to bypass authentication."),
            "explicit_user_statement"
        ));
    }
}
