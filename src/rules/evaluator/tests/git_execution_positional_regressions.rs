use super::*;

#[test]
fn force_push_rule_expands_positional_argument_slices() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c '${@:1}' _ git push --force"#,
            EvaluationVerdict::Block,
        ),
        (r#"bash -c '${@:2}' _ safe true"#, EvaluationVerdict::Allow),
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, expected, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]

fn force_push_rule_updates_positionals_after_shift() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c 'shift; git push "$1"' _ safe --force"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c 'shift; git push "$1"' _ --force safe"#,
            EvaluationVerdict::Allow,
        ),
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, expected, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]

fn force_push_rule_expands_positional_substrings() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c '${1:0}' _ 'git push --force'"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c '${1:4}' _ 'safegit push --force'"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c '${1:4:4}' _ 'safe true'"#,
            EvaluationVerdict::Allow,
        ),
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, expected, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]
fn force_push_rule_resolves_set_valued_positional_forms() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        r#"bash -c 'git push ${1?missing}' _ --force"#,
        r#"bash -c 'git push ${1:?missing}' _ --force"#,
        r#"bash -c 'git push ${1=missing}' _ --force"#,
        r#"bash -c 'git push ${1:=missing}' _ --force"#,
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
        r#"bash -c 'git push ${1?missing}' _"#,
        r#"bash -c 'git push ${1:?missing}' _ ''"#,
        r#"bash -c 'git push ${1=missing}' _"#,
        r#"bash -c 'git push ${1:=missing}' _ ''"#,
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

fn force_push_rule_keeps_possible_positionals_in_concatenated_words() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c 'unknown && set -- force; git push --$1' _ origin"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c 'unknown && set -- safe; git push --$1' _ origin"#,
            EvaluationVerdict::Allow,
        ),
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, expected, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]

fn force_push_rule_resolves_env_function_before_split_string() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c 'env(){ :;}; $1 -S "git push --force"' _ env"#,
            EvaluationVerdict::Allow,
        ),
        (
            r#"bash -c 'env(){ "$@";}; $1 -S "git push --force"' _ env"#,
            EvaluationVerdict::Allow,
        ),
        (
            r#"bash -c '$1 -S "git push --force"' _ env"#,
            EvaluationVerdict::Block,
        ),
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, expected, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]

fn force_push_rule_resolves_alias_function_before_builtin_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c $'shopt -s expand_aliases\nalias(){ :;}\n$1 git=echo\ngit push --force' _ alias"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c $'shopt -s expand_aliases\n$1 git=echo\ngit push --force' _ alias"#,
            EvaluationVerdict::Allow,
        ),
    ] {
        let outcome = evaluate_artifact(
            &artifact,
            &EvaluationInput {
                command: command.into(),
            },
        );
        assert_eq!(outcome.verdict, expected, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }
}

#[test]
fn force_push_rule_restores_caller_positionals_after_sourced_stdin() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'source /dev/stdin safe && git push "$1"' _ --force <<'EOF'
true
EOF"#;
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
fn force_push_rule_does_not_treat_plain_assignment_prefix_as_fallible() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'FOO=x true || git push --force'"#;
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
fn force_push_rule_preserves_top_level_assignment_prefix_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "FOO=x true || git push --force";
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
fn force_push_rule_keeps_readonly_function_names_out_of_variable_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "f(){ :; }; readonly -f f; f=x true || git push --force",
        "readonly -p >/dev/null; FLAG=x true || git push --force",
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

    let command = "readonly -p FLAG; FLAG=x true || git push --force";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
    assert_eq!(outcome.matches.len(), 1, "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");
}

#[test]
fn force_push_rule_honors_source_option_terminator_before_stdin_path() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'source -- /dev/stdin --force' _ origin <<'EOF'
git push "$1"
EOF"#;
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
fn force_push_rule_expands_possible_positionals_in_heredoc() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        r#"bash -c 'unknown && set -- "git push --force"; sh <<EOF
$1
EOF' _ true"#,
        r#"bash -c 'unknown && set -- true; sh <<EOF
$1
EOF' _ "git push --force""#,
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
}
