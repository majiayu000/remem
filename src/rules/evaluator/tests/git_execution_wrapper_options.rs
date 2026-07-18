use super::*;

#[test]
fn force_push_rule_closes_wrapper_option_parsing_gaps() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "git push -od origin +HEAD:main",
        "git push -od origin +main",
        "git() { command git \"$@\"; }; git push --force",
        "env -uHOME git push --force",
        "env A-B=1 git push --force",
        "exec -aNAME git push --force",
        "git --config-env push.default=REMEM push --force",
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
    for command in [
        "git push -o d origin main",
        "git push -od origin main",
        "git push -d origin stale-branch",
        "env -uHOME true",
        "env A-B=1 true",
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]
fn force_push_rule_removes_empty_positional_command_fields() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c '$1 git push --force' _ ''"#;
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
fn force_push_rule_preserves_quoted_default_field_grouping() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c '${1:-"git push --force"}' _"#;
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
fn force_push_rule_selects_static_alternative_positional_values() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        r#"bash -c '${1:+git push --force}' _ x"#,
        r#"bash -c '${1+git push --force}' _ ''"#,
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
    for command in [
        r#"bash -c '${1:+git push --force}' _ ''"#,
        r#"bash -c '${1+git push --force}' _"#,
    ] {
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
}

#[test]
fn force_push_rule_keeps_outer_heredoc_positionals_out_of_child_context() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c 'source /dev/stdin' _ --force <<EOF\ngit push \"$1\"\nEOF";
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
fn force_push_rule_updates_positionals_after_static_set() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'set -- --force; git push "$1"' _ origin"#;
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
fn force_push_rule_binds_source_stdin_arguments() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c 'source /dev/stdin --force' _ origin <<'EOF'\ngit push \"$1\"\nEOF";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}
