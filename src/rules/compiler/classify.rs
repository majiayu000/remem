//! Deterministic, conservative classification of preference text into v1
//! predicates. No LLM, no network. The recognised grammar is intentionally
//! closed so ambiguous natural-language exceptions never become block rules.

/// A predicate template derived from preference text. `conflict_key` groups
/// contradictory preferences so the compiler keeps one authoritative source.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceClassification {
    pub predicate: PreferencePredicate,
    /// Static classifier description; never copied from preference text.
    pub summary: String,
}

const PACKAGE_MANAGER_PREDICATES: &[(&str, &str)] = &[
    (
        "npm",
        r"(^|[ \t\r\n])npm[ \t\r\n]+(install|i|add|ci)([ \t\r\n;&|)]|$)",
    ),
    (
        "yarn",
        r"(^|[ \t\r\n])yarn[ \t\r\n]+(add|install)([ \t\r\n;&|)]|$)",
    ),
    (
        "bun",
        r"(^|[ \t\r\n])bun[ \t\r\n]+(add|install)([ \t\r\n;&|)]|$)",
    ),
    (
        "pnpm",
        r"(^|[ \t\r\n])pnpm[ \t\r\n]+(add|install|i)([ \t\r\n;&|)]|$)",
    ),
];

const FORBIDDEN_COMMANDS: &[(&str, &str, &str)] = &[(
    "git push --force",
    r"(^|[;&|][ \t\r\n]*)git[ \t\r\n]+push[ \t\r\n]+([^ \t\r\n;&|]+[ \t\r\n]+)*(--force|-f)([ \t\r\n;&|]|$)",
    "git-push-force",
)];

const FORBIDDEN_COMMAND_ACTIONS: &[&str] = &["do not run", "don't run", "dont run", "never run"];

const FORBIDDEN_COMMAND_SUFFIXES: &[&str] = &[" in this project", ""];

const PACKAGE_MANAGERS: &[&str] = &["bun", "deno", "npm", "pnpm", "yarn"];

const PACKAGE_DIRECTIVE_SUFFIXES: &[&str] = &[
    ", for package installation commands in this project",
    ", for installing packages in this project",
    ", for package installation commands",
    ", for installing packages",
    ", in this project",
    " in this project",
    "",
];

const TRAILER_DIRECTIVE_SUFFIXES: &[&str] = &[
    " in git commits",
    " on git commits",
    " to git commits",
    " in commits",
    " on commits",
    " to commits",
    "",
];

const TRAILER_ACTIONS: &[&str] = &[
    "do not add",
    "do not include",
    "do not use",
    "don't add",
    "don't include",
    "dont add",
    "dont include",
    "never add",
    "never include",
    "never use",
];

const KNOWN_TRAILERS: &[&str] = &["AI-generated-by", "Co-authored-by", "Generated-by"];

pub fn classify_preference_predicate(text: &str) -> Option<PreferenceClassification> {
    classify_preference_predicates(text).into_iter().next()
}

/// Return every deterministic predicate represented by one closed
/// preference directive. A multi-trailer directive may emit multiple rules.
pub fn classify_preference_predicates(text: &str) -> Vec<PreferenceClassification> {
    if crate::memory_candidate::contains_unsafe_memory_marker(text)
        || crate::adapter::common::redact_sensitive_text(text) != text
    {
        return Vec::new();
    }
    let lower = text.to_lowercase();
    let mut classifications = classify_package_manager(&lower)
        .into_iter()
        .collect::<Vec<_>>();
    classifications.extend(classify_forbidden_command(&lower));
    classifications.extend(classify_commit_trailers(&lower));
    classifications
}

fn classify_forbidden_command(lower: &str) -> Option<PreferenceClassification> {
    for (command, pattern, key) in FORBIDDEN_COMMANDS {
        let is_exact_directive = FORBIDDEN_COMMAND_ACTIONS.iter().any(|action| {
            has_exact_directive(
                lower,
                &format!("{action} {command}"),
                FORBIDDEN_COMMAND_SUFFIXES,
            )
        });
        if is_exact_directive {
            return Some(PreferenceClassification {
                predicate: PreferencePredicate::CommandRegex {
                    pattern: (*pattern).to_string(),
                    conflict_key: format!("forbidden-command:{key}"),
                },
                summary: "Forbidden command".to_string(),
            });
        }
    }
    None
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
                    conflict_key: "package-manager-choice".to_string(),
                },
                summary: "Directed package-manager choice".to_string(),
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
    .any(|directive| has_exact_directive(lower, directive, PACKAGE_DIRECTIVE_SUFFIXES))
}

fn classify_commit_trailers(lower: &str) -> Vec<PreferenceClassification> {
    if !(lower.contains("trailer") || lower.contains("commit")) {
        return Vec::new();
    }

    let mut forbidden = directly_forbidden_trailer_list(lower);
    for trailer in KNOWN_TRAILERS {
        if directly_forbids_term(lower, &trailer.to_lowercase())
            || directly_rejects_trailer_choice(lower, trailer)
        {
            forbidden.push((*trailer).to_string());
        }
    }
    forbidden.sort();
    forbidden.dedup();
    forbidden
        .into_iter()
        .map(|trailer| PreferenceClassification {
            predicate: PreferencePredicate::CommitTrailerForbidden {
                conflict_key: format!("trailer:{}", trailer.to_lowercase()),
                trailer,
            },
            summary: "Forbidden commit trailer".to_string(),
        })
        .collect()
}

fn directly_forbids_term(lower: &str, term: &str) -> bool {
    TRAILER_ACTIONS.iter().any(|action| {
        [
            format!("{action} {term} trailer"),
            format!("{action} the {term} trailer"),
            format!("{action} {term} commit trailer"),
            format!("{action} the {term} commit trailer"),
        ]
        .iter()
        .any(|directive| has_exact_directive(lower, directive, TRAILER_DIRECTIVE_SUFFIXES))
    })
}

fn directly_rejects_trailer_choice(lower: &str, avoided: &str) -> bool {
    KNOWN_TRAILERS.iter().any(|preferred| {
        !preferred.eq_ignore_ascii_case(avoided)
            && [
                format!(
                    "use {}, not {}, in commit trailers",
                    preferred.to_lowercase(),
                    avoided.to_lowercase()
                ),
                format!(
                    "prefer {} over {} in commit trailers",
                    preferred.to_lowercase(),
                    avoided.to_lowercase()
                ),
            ]
            .iter()
            .any(|directive| has_exact_directive(lower, directive, &[""]))
    })
}

fn directly_forbidden_trailer_list(lower: &str) -> Vec<String> {
    let Some(statement) = normalized_single_statement(lower) else {
        return Vec::new();
    };
    for action in TRAILER_ACTIONS {
        let Some(rest) = statement.strip_prefix(&format!("{action} ")) else {
            continue;
        };
        for suffix in TRAILER_DIRECTIVE_SUFFIXES {
            let Some(body) = rest.strip_suffix(suffix) else {
                continue;
            };
            let Some(list) = body
                .strip_suffix(" commit trailers")
                .or_else(|| body.strip_suffix(" trailers"))
            else {
                continue;
            };
            let names = list.split(" or ").collect::<Vec<_>>();
            if names.len() < 2 {
                continue;
            }
            let mut canonical = Vec::with_capacity(names.len());
            for name in names {
                let Some(trailer) = KNOWN_TRAILERS
                    .iter()
                    .find(|trailer| trailer.eq_ignore_ascii_case(name))
                else {
                    canonical.clear();
                    break;
                };
                canonical.push((*trailer).to_string());
            }
            canonical.sort();
            canonical.dedup();
            if canonical.len() >= 2 {
                return canonical;
            }
        }
    }
    Vec::new()
}

fn has_exact_directive(lower: &str, directive: &str, allowed_suffixes: &[&str]) -> bool {
    normalized_single_statement(lower)
        .and_then(|statement| statement.strip_prefix(directive))
        .is_some_and(|suffix| allowed_suffixes.contains(&suffix))
}

fn normalized_single_statement(lower: &str) -> Option<&str> {
    let statement = lower.trim().trim_end_matches(['.', '!', '?']).trim_end();
    if statement
        .chars()
        .any(|ch| [';', ':', '.', '!', '?', '\n', '\r'].contains(&ch))
    {
        return None;
    }
    Some(statement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_pattern(text: &str) -> String {
        match classify_preference_predicate(text)
            .unwrap_or_else(|| panic!("expected classification for {text}"))
            .predicate
        {
            PreferencePredicate::CommandRegex { pattern, .. } => pattern,
            other => panic!("expected command regex for {text}, got {other:?}"),
        }
    }

    #[test]
    fn package_manager_direction_covers_supported_managers() {
        for (text, banned) in [
            ("Use npm, not yarn", "yarn"),
            ("Prefer yarn over bun", "bun"),
            ("Use bun instead of pnpm", "pnpm"),
            ("Use pnpm, not npm", "npm"),
        ] {
            assert!(
                command_pattern(text).contains(banned),
                "{text} banned the wrong manager"
            );
        }
        assert!(classify_preference_predicate("Do not use npm; use pnpm").is_none());
    }

    #[test]
    fn canonical_package_directive_with_project_suffix_classifies() {
        let classification = classify_preference_predicate(
            "Use bun, not npm, for package installation commands in this project.",
        )
        .expect("closed package-manager preference should classify");
        assert!(matches!(
            classification.predicate,
            PreferencePredicate::CommandRegex { ref pattern, .. } if pattern.contains("npm")
        ));
    }

    #[test]
    fn ambiguous_multiclause_preferences_fail_closed() {
        for text in [
            "Never omit Co-authored-by",
            "Prefer bun, but do not forbid npm",
            "Do not add AI-generated-by and use Co-authored-by",
            "Do not avoid npm; use npm instead of yarn",
            "Never, under any circumstances, avoid npm; use npm instead of yarn",
            "Use bun, not npm; unless CI requires npm",
            "Use bun, not npm. Actually use npm for CI",
            "Never add the Co-authored-by trailer. Except for pair-authored commits",
            "There is no good reason, whatsoever, to omit the Co-authored-by trailer",
        ] {
            assert!(
                classify_preference_predicate(text).is_none(),
                "must fail closed: {text}"
            );
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
    fn classifies_forbidden_commit_trailer() {
        let classification =
            classify_preference_predicate("Never add the AI-generated-by trailer to git commits")
                .expect("commit trailer preference should classify");
        assert!(matches!(
            classification.predicate,
            PreferencePredicate::CommitTrailerForbidden { ref trailer, .. }
                if trailer == "AI-generated-by"
        ));
    }

    #[test]
    fn trailer_choice_forbids_only_the_negative_direction() {
        let trailers = classify_preference_predicates(
            "Use AI-generated-by, not Co-authored-by, in commit trailers",
        )
        .into_iter()
        .filter_map(|classification| match classification.predicate {
            PreferencePredicate::CommitTrailerForbidden { trailer, .. } => Some(trailer),
            PreferencePredicate::CommandRegex { .. } => None,
        })
        .collect::<Vec<_>>();
        assert_eq!(trailers, ["Co-authored-by"]);
    }

    #[test]
    fn classifies_each_forbidden_trailer_in_closed_list() {
        let trailers = classify_preference_predicates(
            "Do not add AI-generated-by or Co-authored-by trailers to commits",
        )
        .into_iter()
        .filter_map(|classification| match classification.predicate {
            PreferencePredicate::CommitTrailerForbidden { trailer, .. } => Some(trailer),
            PreferencePredicate::CommandRegex { .. } => None,
        })
        .collect::<Vec<_>>();
        assert_eq!(trailers, ["AI-generated-by", "Co-authored-by"]);
    }

    #[test]
    fn forbidden_command_classifier_is_exact_and_closed() -> anyhow::Result<()> {
        let pattern = command_pattern("Never run git push --force");
        let regex = regex_lite::Regex::new(&pattern)?;
        assert!(regex.is_match("git push --force"));
        assert!(regex.is_match("git push -f"));
        assert!(regex.is_match("git push origin main --force"));
        assert!(regex.is_match("git push origin HEAD:main -f"));
        assert!(regex.is_match("git push --dry-run origin main --force"));
        assert!(regex.is_match("cargo test && git push --force"));
        assert!(!regex.is_match("git push --force-with-lease"));
        assert!(!regex.is_match("git push origin main --force-with-lease"));
        assert!(!regex.is_match("git push -foo"));
        assert!(!regex.is_match("echo git push --force"));

        for text in [
            "Never run rm -rf /",
            "Never run git push --force unless asked",
            "Never run git push --force; use --force-with-lease",
        ] {
            assert!(
                classify_preference_predicate(text).is_none(),
                "must fail closed: {text}"
            );
        }
        Ok(())
    }

    #[test]
    fn ambiguous_or_sensitive_preference_is_not_machine_checkable() {
        assert!(classify_preference_predicate("I like clean code and short functions").is_none());
        assert!(classify_preference_predicate("npm is a package manager").is_none());
        assert!(classify_preference_predicate(
            "Use bun, not npm; the API key is sk-testsecret123456"
        )
        .is_none());
    }
}
