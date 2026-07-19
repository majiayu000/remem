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
