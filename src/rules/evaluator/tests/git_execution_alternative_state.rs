use super::*;

#[test]
fn force_push_rule_executes_alternative_function_definitions_exclusively() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'f(){ git push --force; }; zap(){ builtin unset -f f; }; unknown && zap(){ :; }; zap; f'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_executes_alternative_alias_definitions_exclusively() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'shopt -s expand_aliases\nf(){ git push --force; }\nalias zap=\"builtin unset -f f\"\nunknown && alias zap=:\nzap\nf'";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_does_not_alias_expand_materialized_command_words() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'shopt -s expand_aliases\nalias set=:\nunknown && builtin set -- set safe\n\"$1\" -- \"$2\"\ngit push \"$1\"' _ set --force";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_merges_alternative_alias_shell_states() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'shopt -s expand_aliases\nalias zap=\"set -- --force; set -- safe\"\nunknown && alias zap=\"set -- safe\"\nzap\ngit push \"$1\"' _ origin";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
    assert!(outcome.matches.is_empty(), "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_merges_alternative_function_shell_states() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap(){ builtin trap "git push --force" EXIT; builtin trap - EXIT; }; unknown && trap(){ :; }; trap; exit'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
    assert!(outcome.matches.is_empty(), "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_preserves_uncertain_function_call_skip_path() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'f(){ exit; }; unknown && f; git push --force'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_preserves_uncertain_alias_call_skip_path() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command =
        "bash -c $'shopt -s expand_aliases\nalias zap=exit\nunknown && zap\ngit push --force'";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_tracks_function_local_shift_arguments() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'f(){ shift; }; f safe && git push "$1"' _ --force"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_propagates_terminal_function_shift_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'f(){ shift; }; f && git push "$1"' _ --force"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
    assert!(outcome.matches.is_empty(), "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_keeps_alias_expansion_correlated_with_presence() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'maker(){ shopt -u expand_aliases; alias zap=\"git push --force\"; }\nunknown && maker(){ shopt -s expand_aliases; unalias zap; }\nmaker\nzap'";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
    assert!(outcome.matches.is_empty(), "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_isolates_correlated_command_variants() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'g(){ git push --force; }; f(){ "$1" -f g; }; unknown && set -- :; f "$1"; g' _ unset"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_correlates_command_variant_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && set -- true; "$1" && git push --force' _ false"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_isolates_possible_function_builtin_fallback() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && trap(){ builtin trap "git push --force" EXIT; }; trap - EXIT; exit'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_records_isolated_builtin_fallback_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && true(){ false; }; true && git push --force'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_does_not_override_redirect_failure_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'true >/dev/null/nope || git push --force'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_invalidates_positional_status_on_redirect_failure() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'shift >/dev/null/nope || git push --force' x arg"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_preserves_shell_state_on_redirect_failure() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap "git push --force" EXIT; trap - EXIT >/dev/null/nope; exit'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_branches_redirect_before_function_resolution() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command =
        r#"bash -c 'trap "git push --force" EXIT; f(){ trap - EXIT; }; f >/dev/null/nope; exit'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_preserves_assignment_prefix_status_uncertainty() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'readonly FLAG; FLAG=x true || git push --force'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_branches_assignment_prefix_before_function_resolution() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'readonly FLAG; trap "git push --force" EXIT; f(){ trap - EXIT; }; FLAG=x f; exit'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_collects_exit_traps_before_setup_merge() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap "git push --force" EXIT; FLAG=x exit; trap - EXIT'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_does_not_merge_terminated_shell_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap "shopt -s nocasematch" EXIT; f(){ exit; }; f >/dev/null/nope; case X in x) :;; *) git push --force;; esac'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_collects_traps_for_every_terminated_alternative() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'command true && f(){ trap "git push --force" EXIT; exit; }; f'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_records_builtin_wrapped_guard_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap "git push --force" EXIT; builtin true && exit; trap - EXIT'"#;
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}
