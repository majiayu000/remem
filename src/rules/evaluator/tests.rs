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

fn forbidden_force_push_rule() -> CompiledRule {
    CompiledRule {
        rule_id: "pref-789-1".to_string(),
        source_memory_id: 789,
        reinforcement_count: 5,
        action: RuleAction::Block,
        override_state: RuleOverrideState {
            disabled: false,
            action_override: None,
        },
        predicate: RulePredicate::GitPushForceForbidden {
            message: "Do not force push".to_string(),
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

#[test]
fn legacy_artifact_retains_unicode_word_boundary_semantics() {
    let mut artifact = CompiledRulesArtifact::new(99, vec![package_manager_rule(RuleAction::Warn)]);
    artifact.version = LEGACY_ARTIFACT_VERSION;
    artifact.rules[0].predicate = RulePredicate::CommandRegex {
        pattern: r"(^|\s)npm\s+install\b".to_string(),
        message: "legacy unicode boundary fixture".to_string(),
    };

    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: "npm installβ".to_string(),
        },
    );

    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn commit_trailer_rule_handles_git_global_options() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
    let input = EvaluationInput {
        command: "git -C /repo -c user.email=x commit -m init --trailer AI-generated-by=bot"
            .to_string(),
    };

    let outcome = evaluate_artifact(&artifact, &input);

    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn commit_trailer_rule_ignores_message_text_mentions() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
    let input = EvaluationInput {
        command: "git commit -m 'remove AI-generated-by support'".to_string(),
    };

    let outcome = evaluate_artifact(&artifact, &input);

    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn commit_trailer_rule_requires_git_as_segment_command() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
    let input = EvaluationInput {
        command: "echo git commit --trailer AI-generated-by=bot".to_string(),
    };

    let outcome = evaluate_artifact(&artifact, &input);

    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn commit_trailer_rule_skips_message_option_values() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
    let input = EvaluationInput {
        command: "git commit -m --trailer AI-generated-by=bot".to_string(),
    };

    let outcome = evaluate_artifact(&artifact, &input);

    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn force_push_rule_structurally_matches_exact_options() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "git push --force",
        "git push origin HEAD:main -f",
        "git push -uf origin main",
        "git push -foo origin main",
        "git -c push.default=current push -u origin main --force",
        "cargo test && git push origin main -f",
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.to_string(),
            },
        );
        assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
        assert_eq!(outcome.matches.len(), 1, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]
fn force_push_rule_rejects_values_terminators_and_similar_options() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "git push -- origin -f",
        "git push -o -f origin main",
        "git push --push-option -f origin main",
        "git push -vo -f origin main",
        "git push -of origin main",
        "git push origin main --force-with-lease",
        "echo git push --force",
        "git push origin main",
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.to_string(),
            },
        );
        assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
        assert!(outcome.matches.is_empty(), "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]
fn evaluation_error_fails_open_for_whole_artifact() {
    let artifact = CompiledRulesArtifact::new(
        99,
        vec![
            CompiledRule {
                predicate: RulePredicate::CommandRegex {
                    pattern: "(".to_string(),
                    message: "broken".to_string(),
                },
                ..package_manager_rule(RuleAction::Block)
            },
            forbidden_trailer_rule(),
        ],
    );
    let input = EvaluationInput {
        command: "git commit --trailer AI-generated-by=bot".to_string(),
    };

    let outcome = evaluate_artifact(&artifact, &input);

    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert_eq!(outcome.diagnostics.len(), 1);
    assert!(outcome.diagnostics[0].message.contains("invalid regex"));
}
