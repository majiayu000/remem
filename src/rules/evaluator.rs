use std::path::Path;

use crate::rules::artifact::{CompiledRule, CompiledRulesArtifact, RuleAction, RulePredicate};
use crate::rules::store::{load_artifact_fail_open, ArtifactLoad};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationInput {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationOutcome {
    pub verdict: EvaluationVerdict,
    pub matches: Vec<RuleMatch>,
    pub diagnostics: Vec<EvaluationDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationVerdict {
    Allow,
    Warn,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatch {
    pub rule_id: String,
    pub source_memory_id: i64,
    pub action: RuleAction,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluationDiagnostic {
    pub status: EvaluationDiagnosticStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationDiagnosticStatus {
    Error,
}

pub fn evaluate_artifact(
    artifact: &CompiledRulesArtifact,
    input: &EvaluationInput,
) -> EvaluationOutcome {
    let mut matches = Vec::new();
    let mut diagnostics = Vec::new();

    for rule in &artifact.rules {
        if rule.override_state.disabled {
            continue;
        }
        match rule_matches(rule, input) {
            Ok(true) => matches.push(RuleMatch {
                rule_id: rule.rule_id.clone(),
                source_memory_id: rule.source_memory_id,
                action: rule.effective_action(),
                message: rule.predicate.message().to_string(),
            }),
            Ok(false) => {}
            Err(message) => diagnostics.push(EvaluationDiagnostic {
                status: EvaluationDiagnosticStatus::Error,
                message,
            }),
        }
    }

    EvaluationOutcome {
        verdict: verdict_for_matches(&matches),
        matches,
        diagnostics,
    }
}

pub fn evaluate_artifact_file(
    path: impl AsRef<Path>,
    input: &EvaluationInput,
) -> EvaluationOutcome {
    match load_artifact_fail_open(path) {
        ArtifactLoad::Loaded(artifact) => evaluate_artifact(&artifact, input),
        ArtifactLoad::FailOpen { message, .. } => EvaluationOutcome {
            verdict: EvaluationVerdict::Allow,
            matches: Vec::new(),
            diagnostics: vec![EvaluationDiagnostic {
                status: EvaluationDiagnosticStatus::Error,
                message,
            }],
        },
    }
}

fn rule_matches(rule: &CompiledRule, input: &EvaluationInput) -> Result<bool, String> {
    match &rule.predicate {
        RulePredicate::CommandRegex { pattern, .. } => regex::Regex::new(pattern)
            .map(|regex| regex.is_match(&input.command))
            .map_err(|err| format!("rule {} has invalid regex: {err}", rule.rule_id)),
        RulePredicate::CommitTrailerForbidden { trailer, .. } => {
            Ok(is_git_commit(&input.command) && input.command.contains(trailer))
        }
    }
}

fn is_git_commit(command: &str) -> bool {
    match regex::Regex::new(r"(^|[;&|]\s*)git\s+commit(\s|$)") {
        Ok(regex) => regex.is_match(command),
        Err(_) => false,
    }
}

fn verdict_for_matches(matches: &[RuleMatch]) -> EvaluationVerdict {
    if matches
        .iter()
        .any(|rule_match| rule_match.action == RuleAction::Block)
    {
        EvaluationVerdict::Block
    } else if matches.is_empty() {
        EvaluationVerdict::Allow
    } else {
        EvaluationVerdict::Warn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::artifact::{CompiledRule, RuleOverrideState};
    use crate::rules::test_support::package_manager_rule;

    fn forbidden_trailer_rule() -> CompiledRule {
        CompiledRule {
            rule_id: "pref-456-1".to_string(),
            source_memory_id: 456,
            reinforcement_count: 4,
            action: RuleAction::Block,
            override_state: RuleOverrideState {
                disabled: false,
                action_override: None,
            },
            predicate: RulePredicate::CommitTrailerForbidden {
                trailer: "AI-generated-by".to_string(),
                message: "Do not add AI-generated commit trailers".to_string(),
            },
        }
    }

    #[test]
    fn evaluator_is_deterministic_for_same_input_and_artifact() {
        let artifact = CompiledRulesArtifact::new(
            99,
            vec![
                package_manager_rule(RuleAction::Warn),
                forbidden_trailer_rule(),
            ],
        );
        let input = EvaluationInput {
            command: "npm install && git commit -m init --trailer AI-generated-by=bot".to_string(),
        };

        let first = evaluate_artifact(&artifact, &input);
        let second = evaluate_artifact(&artifact, &input);

        assert_eq!(first, second);
        assert_eq!(first.verdict, EvaluationVerdict::Block);
        assert_eq!(first.matches.len(), 2);
        assert!(first.diagnostics.is_empty());
    }

    #[test]
    fn evaluator_skips_disabled_rules_and_warns_by_default() {
        let mut disabled = package_manager_rule(RuleAction::Warn);
        disabled.override_state.disabled = true;
        let artifact =
            CompiledRulesArtifact::new(99, vec![disabled, package_manager_rule(RuleAction::Warn)]);
        let input = EvaluationInput {
            command: "npm add left-pad".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Warn);
        assert_eq!(outcome.matches.len(), 1);
        assert_eq!(outcome.matches[0].rule_id, "pref-123-1");
    }

    #[test]
    fn invalid_regex_fails_open_for_that_rule() {
        let artifact = CompiledRulesArtifact::new(
            99,
            vec![CompiledRule {
                predicate: RulePredicate::CommandRegex {
                    pattern: "(".to_string(),
                    message: "broken".to_string(),
                },
                ..package_manager_rule(RuleAction::Block)
            }],
        );
        let input = EvaluationInput {
            command: "npm install".to_string(),
        };

        let outcome = evaluate_artifact(&artifact, &input);

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
        assert!(outcome.matches.is_empty());
        assert_eq!(outcome.diagnostics.len(), 1);
        assert!(outcome.diagnostics[0].message.contains("invalid regex"));
    }
}
