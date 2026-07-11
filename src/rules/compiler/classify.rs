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

/// The classification result for one derived predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceClassification {
    pub predicate: PreferencePredicate,
    /// Static classifier description. This never contains preference text and
    /// is not used as an artifact message.
    pub summary: String,
}

struct PackageManager {
    name: &'static str,
    install_pattern: &'static str,
}

/// Closed v1 package-manager vocabulary. A rule is emitted only when one
/// manager is positively selected and a different manager is explicitly
/// rejected.
const PACKAGE_MANAGERS: &[PackageManager] = &[
    PackageManager {
        name: "npm",
        install_pattern: r"(^|\s)npm\s+(install|i|add|ci)\b",
    },
    PackageManager {
        name: "yarn",
        install_pattern: r"(^|\s)yarn\s+(add|install)\b",
    },
    PackageManager {
        name: "bun",
        install_pattern: r"(^|\s)bun\s+(add|install)\b",
    },
    PackageManager {
        name: "pnpm",
        install_pattern: r"(^|\s)pnpm\s+(add|install|i)\b",
    },
];

/// Commit trailers a preference may forbid.
const KNOWN_TRAILERS: &[&str] = &["AI-generated-by", "Co-authored-by", "Generated-by"];

pub fn classify_preference_predicate(text: &str) -> Option<PreferenceClassification> {
    classify_preference_predicates(text).into_iter().next()
}

/// Return every deterministic v1 predicate represented by `text`. Multiple
/// forbidden trailers may be carried by one preference and receive stable rule
/// suffixes in the compiler.
pub fn classify_preference_predicates(text: &str) -> Vec<PreferenceClassification> {
    let lower = text.to_lowercase();
    let mut classifications = classify_package_managers(&lower);
    classifications.extend(classify_commit_trailers(&lower));
    classifications
}

fn classify_package_managers(lower: &str) -> Vec<PreferenceClassification> {
    let tokens = directional_tokens(lower);
    let mut preferred = std::collections::BTreeSet::new();
    let mut rejected = std::collections::BTreeSet::new();

    for (index, token) in tokens.iter().enumerate() {
        let Some(manager) = PACKAGE_MANAGERS
            .iter()
            .find(|manager| manager.name == token)
        else {
            continue;
        };
        if manager_is_rejected(&tokens, index) {
            rejected.insert(manager.name);
        } else if manager_is_preferred(&tokens, index) {
            preferred.insert(manager.name);
        }
    }

    // More than one positively selected manager is ambiguous. A rejected
    // manager must be distinct from the single selected manager.
    if preferred.len() != 1 {
        return Vec::new();
    }
    let selected = preferred.iter().next().copied();
    PACKAGE_MANAGERS
        .iter()
        .filter(|manager| rejected.contains(manager.name) && Some(manager.name) != selected)
        .map(|manager| PreferenceClassification {
            predicate: PreferencePredicate::CommandRegex {
                pattern: manager.install_pattern.to_string(),
                conflict_key: "package-manager-choice".to_string(),
            },
            summary: "Directed package-manager choice".to_string(),
        })
        .collect()
}

fn classify_commit_trailers(lower: &str) -> Vec<PreferenceClassification> {
    if !(lower.contains("trailer") || lower.contains("commit")) {
        return Vec::new();
    }
    let clauses = direction_clauses(lower);
    KNOWN_TRAILERS
        .iter()
        .filter(|trailer| {
            let trailer = trailer.to_lowercase();
            clauses
                .iter()
                .any(|clause| trailer_is_rejected(clause, &trailer))
        })
        .map(|trailer| PreferenceClassification {
            predicate: PreferencePredicate::CommitTrailerForbidden {
                trailer: (*trailer).to_string(),
                conflict_key: format!("trailer:{}", trailer.to_lowercase()),
            },
            summary: "Forbidden commit trailer".to_string(),
        })
        .collect()
}

fn directional_tokens(text: &str) -> Vec<String> {
    text.replace("don't", "dont")
        .replace("do not", "do-not")
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

fn manager_is_rejected(tokens: &[String], index: usize) -> bool {
    let previous = |offset: usize| index.checked_sub(offset).and_then(|at| tokens.get(at));
    matches!(
        previous(1).map(String::as_str),
        Some("not" | "avoid" | "never" | "without" | "no" | "ban" | "forbid" | "do-not" | "dont")
    ) || (previous(1).is_some_and(|token| token == "use")
        && matches!(
            previous(2).map(String::as_str),
            Some("not" | "never" | "avoid" | "do-not" | "dont")
        ))
        || (previous(1).is_some_and(|token| token == "than")
            && previous(2).is_some_and(|token| token == "rather"))
        || previous(1).is_some_and(|token| token == "over")
        || (previous(1).is_some_and(|token| token == "of")
            && previous(2).is_some_and(|token| token == "instead"))
}

fn manager_is_preferred(tokens: &[String], index: usize) -> bool {
    let previous = |offset: usize| index.checked_sub(offset).and_then(|at| tokens.get(at));
    let next = |offset: usize| tokens.get(index + offset);
    let preceded_by_choice = matches!(
        previous(1).map(String::as_str),
        Some("use" | "prefer" | "choose" | "favor" | "favour")
    );
    let choice_is_negated = matches!(
        previous(2).map(String::as_str),
        Some("not" | "never" | "avoid" | "do-not" | "dont")
    );
    (preceded_by_choice && !choice_is_negated)
        || next(1).is_some_and(|token| token == "over")
        || (next(1).is_some_and(|token| token == "instead")
            && next(2).is_some_and(|token| token == "of"))
        || (next(1).is_some_and(|token| token == "rather")
            && next(2).is_some_and(|token| token == "than"))
}

fn direction_clauses(lower: &str) -> Vec<String> {
    lower
        .replace(" however ", ";")
        .replace(" but ", ";")
        .split([';', '\n'])
        .map(str::trim)
        .filter(|clause| !clause.is_empty())
        .map(str::to_string)
        .collect()
}

fn clause_has_negative_direction(clause: &str) -> bool {
    let tokens = directional_tokens(clause);
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "avoid"
                | "never"
                | "without"
                | "forbid"
                | "forbidden"
                | "ban"
                | "banned"
                | "no"
                | "dont"
                | "do-not"
        )
    }) || tokens.windows(2).any(|pair| {
        pair.first().is_some_and(|token| token == "not")
            && pair
                .get(1)
                .is_some_and(|token| matches!(token.as_str(), "add" | "append" | "include" | "use"))
    })
}

fn trailer_is_rejected(clause: &str, trailer: &str) -> bool {
    let tokens = directional_tokens(clause);
    let has_trailer = tokens.iter().any(|token| token == trailer);
    has_trailer
        && (clause_has_negative_direction(clause)
            || tokens
                .iter()
                .enumerate()
                .any(|(index, token)| token == trailer && manager_is_rejected(&tokens, index)))
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
                assert_eq!(conflict_key, "package-manager-choice");
            }
            other => panic!("expected command regex, got {other:?}"),
        }
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
    }

    #[test]
    fn package_manager_direction_covers_supported_managers() {
        for (text, banned) in [
            ("Use npm, not yarn", "yarn"),
            ("Prefer yarn over bun", "bun"),
            ("Use bun instead of pnpm", "pnpm"),
            ("Do not use npm; use pnpm", "npm"),
        ] {
            let classification = classify_preference_predicate(text)
                .unwrap_or_else(|| panic!("expected classification for {text}"));
            match classification.predicate {
                PreferencePredicate::CommandRegex { pattern, .. } => {
                    assert!(pattern.contains(banned), "{text} banned the wrong manager");
                }
                other => panic!("expected command regex for {text}, got {other:?}"),
            }
        }
    }

    #[test]
    fn positive_commit_trailer_direction_is_not_forbidden() {
        assert!(
            classify_preference_predicate("Use the Co-authored-by trailer for paired commits")
                .is_none()
        );
    }

    #[test]
    fn trailer_choice_forbids_only_the_negative_direction() {
        let classifications = classify_preference_predicates(
            "Use AI-generated-by, not Co-authored-by, in commit trailers",
        );
        let trailers = classifications
            .into_iter()
            .filter_map(|classification| match classification.predicate {
                PreferencePredicate::CommitTrailerForbidden { trailer, .. } => Some(trailer),
                PreferencePredicate::CommandRegex { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(trailers, vec!["Co-authored-by"]);
    }

    #[test]
    fn classifies_each_forbidden_trailer_in_a_multi_trailer_preference() {
        let classifications = classify_preference_predicates(
            "Do not add AI-generated-by or Co-authored-by trailers to commits",
        );
        let trailers = classifications
            .into_iter()
            .filter_map(|classification| match classification.predicate {
                PreferencePredicate::CommitTrailerForbidden { trailer, .. } => Some(trailer),
                PreferencePredicate::CommandRegex { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(trailers, vec!["AI-generated-by", "Co-authored-by"]);
    }
}
