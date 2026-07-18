use super::*;
use crate::rules::artifact::{CompiledRule, RuleOverrideState};
use crate::rules::test_support::package_manager_rule;

mod git_execution;
mod git_execution_wrapper_options;

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
        "git --config-env=push.default=REMEM_GIT_CONFIG push origin main --force",
        "cargo test && git push origin main -f",
        "git push \"--force\"",
        "{ git push --force; }",
        "! { git push --force; }",
        "echo \"$(git push --force)\"",
        "echo $(( $(git push --force) + 1 ))",
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
fn force_push_rule_matches_mirror_and_command_wrappers() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "git push --mirror origin",
        "env GIT_SSH_COMMAND=ssh git push --force",
        "env -i -u HOME command -p git push --mirror origin",
        "command git push --force",
        "command -- git push --force",
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
fn force_push_rule_keeps_non_executing_wrappers_inert() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "command -v git push --force",
        "command -V git push --force",
        "command echo git push --force",
        "env NOTE=example echo git push --force",
        "git push -- --mirror",
        "git push --no-mirror origin",
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
fn force_push_rule_does_not_execute_function_definitions() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "f() { git push --force; }",
        "function deploy { git push --mirror origin; }",
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
fn force_push_rule_decodes_static_ansi_c_words() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "$'git' push --force",
        "git push $'--force'",
        "$'\\x67it' push $'--\\x66orce'",
        "g$'\\151t' push --force",
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
fn force_push_rule_traverses_commands_inside_arithmetic_expansion() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "{ echo $(( $(git push --force) + 1 )); }",
        "(( $(git push --force) + 1 ))",
        "for (( i = $(git push --force); i < 1; i++ )); do echo ok; done",
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
fn force_push_rule_traverses_expanding_shell_contexts() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "FOO=$(git push --force) true",
        "cat <<EOF\n$(git push --force)\nEOF",
        "bash -c 'git push --force'",
        "/bin/sh -ec 'git push --mirror origin'",
        "env FLAG=1 command bash -lc 'git push --force'",
        "git push --{force,force}",
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
fn force_push_rule_keeps_non_expanding_shell_text_inert() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "cat <<'EOF'\n$(git push --force)\nEOF",
        "bash -c \"$PAYLOAD\"",
        "git push '--{force,force}'",
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
fn force_push_rule_keeps_scanning_after_large_static_expansion() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "printf '%s\\n' {1..257}; git push --force",
        "printf '%s\\n' {1..17} {1..17}; git push --force",
        "printf '%s\\n' {9223372036854775806..9223372036854775807}; git push --force",
        "git push {1..257} --force",
        "git push origin main --force {1..300}",
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
fn force_push_rule_unwraps_exec_and_env_documented_forms() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "exec git push --force",
        "exec -a argv0 git push --force",
        "exec -cl git push --force",
        "exec -- git push --force",
        "env - git push --force",
        "env -S 'git push --force'",
        "env -i -S 'git push --force'",
        "env -u HOME -S 'git push --force'",
        "env --chdir /tmp -S 'git push --force'",
        "env -S 'FOO=1 git push --force'",
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
    for command in ["exec -a git push --force", "env -S '$PAYLOAD'"] {
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
fn force_push_rule_recognizes_append_assignment_prefixes() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "FOO+=bar git push --force",
        "FOO+=bar BAR=1 git push --force",
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
fn force_push_rule_reparses_static_eval_arguments() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "eval 'git push --force'",
        "eval git push --force",
        "eval 'git push' --force",
        "command eval 'git push --force'",
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
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: "eval \"$PAYLOAD\"".to_string(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn force_push_rule_evaluates_function_bodies_on_static_invocation() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "f() { git push --force; }; f",
        "function deploy { git push --mirror origin; }\ndeploy",
        "f() { git push --force; }; f --now",
        "f() { f; g; }; g() { git push --force; }; f",
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
    for command in [
        "f() { git push --force; }; command f",
        "f() { git push --force; }; g",
        "f() { f; }; f",
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
fn force_push_rule_parses_shell_stdin_heredocs() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "sh <<'EOF'\ngit push --force\nEOF",
        "bash <<EOF\ngit push --force\nEOF",
        "/bin/sh <<-'EOF'\n\tgit push --force\n\tEOF",
        "bash <<< 'git push --force'",
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
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: "sh script.sh <<'EOF'\ngit push --force\nEOF".to_string(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn force_push_rule_traverses_parameter_expansion_payloads() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "echo ${UNSET:-$(git push --force)}",
        "echo ${UNSET:=$(git push --force)}",
        "echo ${SET:+$(git push --force)}",
        "echo ${VAR%$(git push --force)}",
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
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: "echo ${UNSET:-safe}".to_string(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn force_push_rule_skips_shell_options_with_arguments_before_dash_c() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "bash --noprofile --rcfile /dev/null -c 'git push --force'",
        "bash --init-file /dev/null -c 'git push --force'",
        "bash -O extglob -c 'git push --force'",
        "bash +O extglob -c 'git push --force'",
        "bash -o errexit -c 'git push --force'",
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
fn force_push_rule_preserves_critical_variants_past_materialization_limit() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let alternatives = std::iter::repeat_n("force", 257)
        .collect::<Vec<_>>()
        .join(",");
    let suffixes = (1..=256)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",");
    for command in [
        format!("git push --{{{alternatives}}}"),
        format!("git push --{{f,x}}{{orce,{suffixes}}}"),
    ] {
        let outcome = evaluate_artifact(&artifact, &EvaluationInput { command });

        assert_eq!(outcome.verdict, EvaluationVerdict::Block);
        assert_eq!(outcome.matches.len(), 1);
        assert!(outcome.diagnostics.is_empty());
    }
}

#[test]
fn package_manager_rule_matches_adjacent_redirections() {
    let artifact = CompiledRulesArtifact::new(99, vec![package_manager_rule(RuleAction::Block)]);
    for command in ["npm install>log", "npm install<input", "npm install 2>>log"] {
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
        "echo {git push --force}",
        "echo $((1 << 2))",
        "cat <<EOF\ngit push --force\nEOF",
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

#[test]
fn bash_parse_errors_fail_open_with_a_diagnostic() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in ["git push --force '", "echo { git push --force; }"] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.to_string(),
            },
        );

        assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
        assert!(outcome.matches.is_empty(), "{command}");
        assert_eq!(outcome.diagnostics.len(), 1, "{command}");
        assert!(
            outcome.diagnostics[0].message.contains("parse error"),
            "{command}"
        );
    }
}
