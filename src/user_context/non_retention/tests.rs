use super::block_reason;

#[test]
fn preventive_constraints_do_not_bypass_harmful_or_secret_rejection() {
    for (claim, source) in [
        (
            "User requires agents never to bypass authentication.",
            "I require agents never to bypass authentication.",
        ),
        (
            "User requires phishing prevention.",
            "I require phishing prevention.",
        ),
        ("用户要求不要绕过认证。", "我要求不要绕过认证。"),
        ("用户要求防范恶意软件。", "我要求防范恶意软件。"),
    ] {
        assert_eq!(
            block_reason(claim, Some(source), "explicit_user_statement"),
            None,
            "{claim}"
        );
    }
    for text in [
        "User wants to bypass authentication.",
        "User does not want to never bypass authentication.",
        "User never bypass authentication unless convenient.",
        "\"Never bypass authentication.\"",
        "User works on malware analysis.",
        "用户想绕过认证。",
        "用户不是不要绕过认证。",
        "用户不要绕过认证，除非方便。",
        "“不要绕过认证。”",
        "用户从事恶意软件分析。",
    ] {
        assert_eq!(
            block_reason(text, Some(text), "explicit_user_statement"),
            Some("illegal_or_harmful_content"),
            "{text}"
        );
    }
    for (claim, source) in [
        (
            "Never bypass authentication.",
            "Never bypass authentication. The API key is sk-testsecret123456.",
        ),
        (
            "不要绕过认证。",
            "不要绕过认证。 API key is sk-testsecret123456.",
        ),
    ] {
        assert_eq!(
            block_reason(claim, Some(source), "explicit_user_statement"),
            Some("secret_like_content")
        );
    }
    assert_eq!(
        block_reason(
            "Never bypass authentication.",
            Some("The README says: never bypass authentication."),
            "explicit_user_statement"
        ),
        Some("illegal_or_harmful_content")
    );
}

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
    assert_eq!(
        block_reason(
            "User lives in Paris.",
            Some("From the web page, the user lives in Paris."),
            "explicit_user_statement"
        ),
        Some("unapproved_external_source")
    );
    assert_eq!(
        block_reason(
            "User thinks SQLite is a single-file database.",
            Some("I think SQLite is a single-file database."),
            "explicit_user_statement"
        ),
        Some("general_knowledge_content")
    );
}
