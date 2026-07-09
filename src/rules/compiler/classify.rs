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

/// Known package managers that a preference may ask the agent to avoid.
const BANNED_MANAGERS: &[(&str, &str)] = &[
    ("npm", r"(^|\s)npm\s+(install|i|add|ci)\b"),
    ("yarn", r"(^|\s)yarn\s+(add|install)\b"),
];

/// Words signalling the preference expresses avoidance / a hard choice.
const AVOIDANCE_MARKERS: &[&str] = &[
    "not", "instead", "over", "don't", "dont", "do not", "avoid", "never", "prefer", "use ", "no ",
    "forbid", "ban", "without",
];

/// Commit trailers a preference may forbid.
const KNOWN_TRAILERS: &[&str] = &[
    "AI-generated-by",
    "Co-Authored-By",
    "Generated-by",
    "Co-authored-by",
];

pub fn classify_preference_predicate(text: &str) -> Option<PreferenceClassification> {
    let lower = text.to_lowercase();
    if let Some(classification) = classify_package_manager(text, &lower) {
        return Some(classification);
    }
    classify_commit_trailer(text, &lower)
}

fn classify_package_manager(original: &str, lower: &str) -> Option<PreferenceClassification> {
    if !has_avoidance_marker(lower) {
        return None;
    }
    let alternatives = ["bun", "pnpm", "deno", "yarn"];
    for (banned, pattern) in BANNED_MANAGERS {
        if !lower.contains(banned) {
            continue;
        }
        // Only compile a genuine "use X, not <banned>" choice: require a
        // different preferred manager to be named. Prevents banning a manager
        // that is merely mentioned in passing.
        let has_preferred_alternative = alternatives
            .iter()
            .any(|alt| alt != banned && lower.contains(alt));
        if has_preferred_alternative {
            return Some(PreferenceClassification {
                predicate: PreferencePredicate::CommandRegex {
                    pattern: (*pattern).to_string(),
                    conflict_key: format!("package-manager:{banned}"),
                },
                summary: normalize_summary(original),
            });
        }
    }
    None
}

fn classify_commit_trailer(original: &str, lower: &str) -> Option<PreferenceClassification> {
    if !has_avoidance_marker(lower) {
        return None;
    }
    if !(lower.contains("trailer") || lower.contains("commit")) {
        return None;
    }
    for trailer in KNOWN_TRAILERS {
        if lower.contains(&trailer.to_lowercase()) {
            return Some(PreferenceClassification {
                predicate: PreferencePredicate::CommitTrailerForbidden {
                    trailer: (*trailer).to_string(),
                    conflict_key: format!("trailer:{}", trailer.to_lowercase()),
                },
                summary: normalize_summary(original),
            });
        }
    }
    None
}

fn has_avoidance_marker(lower: &str) -> bool {
    AVOIDANCE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn normalize_summary(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX: usize = 120;
    if collapsed.chars().count() > MAX {
        let truncated: String = collapsed.chars().take(MAX).collect();
        format!("{truncated}…")
    } else {
        collapsed
    }
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
                assert_eq!(conflict_key, "package-manager:npm");
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
}
