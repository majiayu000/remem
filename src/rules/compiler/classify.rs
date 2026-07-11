//! Deterministic, conservative classification of preference text into v1
//! predicates. No LLM, no network. The set of recognised patterns is
//! intentionally narrow to keep false positives near zero; new predicate kinds
//! require a spec update (see docs/specs/preference-rule-compilation/TECH.md).

/// A predicate template derived from preference text. `conflict_key` groups
/// contradictory preferences (e.g. two package-manager rules) so the compiler
/// can keep only the newest source memory on conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferencePredicate {
    CommandRegex {
        pattern: String,
        conflict_key: String,
    },
    CommitTrailerForbidden {
        trailer: String,
        conflict_key: String,
    },
}

impl PreferencePredicate {
    pub fn conflict_key(&self) -> String {
        match self {
            PreferencePredicate::CommandRegex { conflict_key, .. }
            | PreferencePredicate::CommitTrailerForbidden { conflict_key, .. } => {
                conflict_key.clone()
            }
        }
    }
}

/// The classification result: the predicate template plus a short human summary
/// used to build the rule message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceClassification {
    pub predicate: PreferencePredicate,
    pub summary: String,
}

/// Package managers with v1 command predicates.
const PACKAGE_MANAGER_PREDICATES: &[(&str, &str)] = &[
    ("npm", r"(^|\s)npm\s+(install|i|add|ci)\b"),
    ("yarn", r"(^|\s)yarn\s+(add|install)\b"),
];

const PACKAGE_MANAGERS: &[&str] = &["bun", "deno", "npm", "pnpm", "yarn"];

/// Commit trailers a preference may forbid.
const KNOWN_TRAILERS: &[&str] = &[
    "AI-generated-by",
    "Co-Authored-By",
    "Generated-by",
    "Co-authored-by",
];

pub fn classify_preference_predicate(text: &str) -> Option<PreferenceClassification> {
    if crate::memory_candidate::contains_unsafe_memory_marker(text)
        || crate::adapter::common::redact_sensitive_text(text) != text
    {
        return None;
    }
    let lower = text.to_lowercase();
    if let Some(classification) = classify_package_manager(&lower) {
        return Some(classification);
    }
    classify_commit_trailer(&lower)
}

fn classify_package_manager(lower: &str) -> Option<PreferenceClassification> {
    for (avoided, pattern) in PACKAGE_MANAGER_PREDICATES {
        for preferred in PACKAGE_MANAGERS {
            if preferred == avoided || !directly_prefers_manager(lower, preferred, avoided) {
                continue;
            }
            return Some(PreferenceClassification {
                predicate: PreferencePredicate::CommandRegex {
                    pattern: (*pattern).to_string(),
                    // All package-manager choices conflict. Otherwise two
                    // opposite choices could compile into mutually exclusive
                    // bans merely because they avoid different managers.
                    conflict_key: "package-manager".to_string(),
                },
                summary: format!("avoid {avoided} package-manager commands"),
            });
        }
    }
    None
}

fn directly_prefers_manager(lower: &str, preferred: &str, avoided: &str) -> bool {
    [
        format!("use {preferred}, not {avoided}"),
        format!("use {preferred} instead of {avoided}"),
        format!("use {preferred} rather than {avoided}"),
        format!("prefer {preferred} over {avoided}"),
    ]
    .iter()
    .any(|directive| has_unambiguous_directive(lower, directive))
}

fn classify_commit_trailer(lower: &str) -> Option<PreferenceClassification> {
    if !(lower.contains("trailer") || lower.contains("commit")) {
        return None;
    }
    for trailer in KNOWN_TRAILERS {
        let trailer_lower = trailer.to_lowercase();
        if directly_forbids_term(lower, &trailer_lower) {
            return Some(PreferenceClassification {
                predicate: PreferencePredicate::CommitTrailerForbidden {
                    trailer: (*trailer).to_string(),
                    conflict_key: format!("trailer:{trailer_lower}"),
                },
                summary: format!("do not add the {trailer} commit trailer"),
            });
        }
    }
    None
}

fn directly_forbids_term(lower: &str, term: &str) -> bool {
    [
        format!("do not add {term}"),
        format!("do not add the {term}"),
        format!("do not include {term}"),
        format!("do not include the {term}"),
        format!("do not use {term}"),
        format!("do not use the {term}"),
        format!("don't add {term}"),
        format!("don't add the {term}"),
        format!("don't include {term}"),
        format!("don't include the {term}"),
        format!("dont add {term}"),
        format!("dont add the {term}"),
        format!("dont include {term}"),
        format!("dont include the {term}"),
        format!("never add {term}"),
        format!("never add the {term}"),
        format!("never include {term}"),
        format!("never include the {term}"),
        format!("never use {term}"),
        format!("never use the {term}"),
    ]
    .iter()
    .any(|directive| has_unambiguous_directive(lower, directive))
}

fn has_unambiguous_directive(lower: &str, directive: &str) -> bool {
    lower
        .split([';', '.', '!', '?', '\n', '\r'])
        .map(str::trim)
        .any(|clause| {
            let Some(remainder) = clause.strip_prefix(directive) else {
                return false;
            };
            if remainder
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-')
            {
                return false;
            }
            !remainder
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'')
                .filter(|token| !token.is_empty())
                .any(|token| {
                    matches!(
                        token,
                        "not" | "never" | "no" | "cannot" | "cant" | "dont" | "unless" | "except"
                    ) || token.ends_with("n't")
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_use_bun_not_npm() {
        let classification =
            classify_preference_predicate("Use bun, not npm, for installing packages")
                .expect("package-manager preference should classify");
        match classification.predicate {
            PreferencePredicate::CommandRegex {
                pattern,
                conflict_key,
            } => {
                assert!(pattern.contains("npm"));
                assert_eq!(conflict_key, "package-manager");
            }
            other => panic!("expected command regex, got {other:?}"),
        }
    }

    #[test]
    fn classifies_the_explicitly_avoided_manager_not_the_first_manager() {
        let Some(classification) = classify_preference_predicate("Use npm, not yarn") else {
            panic!("explicit package-manager direction should classify");
        };
        match classification.predicate {
            PreferencePredicate::CommandRegex {
                pattern,
                conflict_key,
            } => {
                assert!(pattern.contains("yarn"));
                assert!(!pattern.contains("npm"));
                assert_eq!(conflict_key, "package-manager");
            }
            other => panic!("expected command regex, got {other:?}"),
        }
    }

    #[test]
    fn positive_commit_trailer_instruction_is_not_inverted() {
        assert!(
            classify_preference_predicate("Always add the Co-Authored-By commit trailer").is_none()
        );
        assert!(
            classify_preference_predicate("Use the AI-generated-by trailer on commits").is_none()
        );
    }

    #[test]
    fn negated_avoidance_is_not_inverted() {
        for text in [
            "Do not avoid npm; use npm instead of yarn",
            "Never avoid npm; use npm instead of yarn",
            "There is no reason to avoid npm; use npm instead of yarn",
            "There is no good reason whatsoever to avoid npm; use npm instead of yarn",
            "You shouldn't avoid npm; use npm instead of yarn",
            "Never, under any circumstances, avoid npm; use npm instead of yarn",
        ] {
            let Some(classification) = classify_preference_predicate(text) else {
                panic!("the explicit yarn avoidance should classify: {text}");
            };
            match classification.predicate {
                PreferencePredicate::CommandRegex { pattern, .. } => {
                    assert!(pattern.contains("yarn"), "unexpected rule for {text}");
                    assert!(!pattern.contains("npm"), "inverted rule for {text}");
                }
                other => panic!("expected command regex for {text}, got {other:?}"),
            }
        }

        for text in [
            "Do not avoid the Co-Authored-By commit trailer; always add it",
            "Never forbid the Co-Authored-By commit trailer; always add it",
            "There is no reason to omit the Co-Authored-By commit trailer",
            "There is no good reason whatsoever to omit the Co-Authored-By commit trailer",
            "There is no good reason, whatsoever, to omit the Co-Authored-By commit trailer",
        ] {
            assert!(
                classify_preference_predicate(text).is_none(),
                "negated trailer avoidance must fail closed: {text}"
            );
        }
    }

    #[test]
    fn sensitive_preference_is_not_machine_checkable() {
        assert!(classify_preference_predicate(
            "Use bun, not npm; the API key is sk-testsecret123456"
        )
        .is_none());
    }

    #[test]
    fn classifies_forbidden_commit_trailer() {
        let classification =
            classify_preference_predicate("Never add the AI-generated-by trailer to git commits")
                .expect("commit trailer preference should classify");
        match classification.predicate {
            PreferencePredicate::CommitTrailerForbidden {
                trailer,
                conflict_key,
            } => {
                assert_eq!(trailer, "AI-generated-by");
                assert_eq!(conflict_key, "trailer:ai-generated-by");
            }
            other => panic!("expected commit trailer, got {other:?}"),
        }
    }

    #[test]
    fn ambiguous_preference_is_not_machine_checkable() {
        assert!(classify_preference_predicate("I like clean code and short functions").is_none());
        // Mentions npm but with no preferred alternative or avoidance choice.
        assert!(classify_preference_predicate("npm is a package manager").is_none());
        assert!(classify_preference_predicate(
            "Never, under any circumstances, avoid npm; yarn remains available"
        )
        .is_none());
        assert!(classify_preference_predicate("Not only npm but yarn").is_none());
        assert!(classify_preference_predicate("Never use bun instead of npm").is_none());
    }
}
