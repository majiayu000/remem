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

#[test]
fn force_push_rule_does_not_reclassify_expanded_assignment_words() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c '$1=1 git push --force' _ FOO"#;
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
fn force_push_rule_does_not_alias_expand_positional_command_names() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'shopt -s expand_aliases\nalias git=echo\n$1 push --force' _ git";
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
fn force_push_rule_preserves_here_string_positionals_as_source_text() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c 'sh <<< $1' _ $'echo SAFE\ngit push --force'";
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
fn force_push_rule_keeps_expanded_env_split_wrapper_semantics() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c '$1 -S "git push --force"' _ env"#;
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
fn force_push_rule_preserves_quoted_all_argument_fields() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c '"$@"' _ git push --force"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c '"$@"' _ 'git push --force'"#,
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
fn force_push_rule_keeps_possible_set_positionals() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        r#"bash -c 'unknown && set -- --force; git push "$1"' _ origin"#,
        r#"bash -c 'unknown && set -- origin; git push "$1"' _ --force"#,
        r#"bash -c 'unknown && set -- force; git push "--${1}"' _ delete"#,
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

#[test]
fn force_push_rule_restores_positionals_after_child_scopes() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        r#"bash -c '(set -- --force); git push "$1"' _ origin"#,
        r#"bash -c '$(set -- --force); git push "$1"' _ origin"#,
        r#"bash -c 'set -- --force | true; git push "$1"' _ origin"#,
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
fn force_push_rule_resolves_alias_before_set_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command =
        "bash -c $'shopt -s expand_aliases\nalias set=echo\nset -- --force\ngit push \"$1\"' _ origin";
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
fn force_push_rule_keeps_possible_command_positions_separate() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c 'unknown && set -- git push --force; "$@"' _ true"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c 'unknown && set -- g; "${1}it" push --force' _ true"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c 'false && set -- git push --force; "$@"' _ true"#,
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
fn force_push_rule_expands_positional_collection_slices() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c '${@:1}' _ git push --force"#;
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
fn force_push_rule_updates_positionals_after_static_shift() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'shift; git push "$1"' _ safe --force"#;
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
fn force_push_rule_expands_static_positional_substrings() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c '${1:0}' _ 'git push --force'"#;
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
fn force_push_rule_resolves_trap_function_before_builtin_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap(){ :; }; $1 '\''git push --force'\'' EXIT; exit' _ trap"#;
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
fn force_push_rule_updates_positionals_after_set_dash() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'set - --force; git push "$1"' _ origin"#;
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
fn force_push_rule_keeps_possible_force_flags_on_separate_paths() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        r#"bash -c 'unknown && set -- --no-force; git push "$1"' _ --force"#,
        r#"bash -c 'unknown && set -- --force; git push "$1"' _ --no-force"#,
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

#[test]
fn force_push_rule_includes_shell_zero_in_collection_slice() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c '${@:0}' git push --force"#,
            EvaluationVerdict::Block,
        ),
        (
            r#"bash -c '"${*:0}"' git push --force"#,
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
fn force_push_rule_tracks_static_shift_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c 'shift 2 && git push "$1"' _ --force"#,
            EvaluationVerdict::Allow,
        ),
        (
            r#"bash -c 'shift 2 || git push "$1"' _ --force"#,
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
fn force_push_rule_resolves_unset_alias_before_function_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'shopt -s expand_aliases\nalias unset=:\nf(){ git push --force; }\nunset -f f\nf'";
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
fn force_push_rule_applies_correlated_set_paths_once() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && set -- safe; set -- "$1"; git push "$1"' _ --force"#;
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
fn force_push_rule_correlates_shift_status_with_arguments() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && set -- safe safe; shift 2 && git push "$1"' _ --force"#;
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
fn force_push_rule_preserves_all_path_alias_presence() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c $'shopt -s expand_aliases\nalias unset=:\nunknown && alias unset=:\nf(){ git push --force; }\nunset -f f\nf'";
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
fn force_push_rule_preserves_all_path_function_presence() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'trap(){ builtin trap "git push --force" EXIT; }; unknown && trap(){ builtin trap "git push --force" EXIT; }; trap - EXIT; exit'"#;
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
fn force_push_rule_preserves_skipped_correlated_positional_changes() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && set -- 0 safe; another_unknown && shift "$1"; git push "$2"' _ 1 --force"#;
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
fn force_push_rule_inverts_correlated_shift_status() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && set -- x y; ! shift 2 && git push "$1"' _ --force"#;
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
fn force_push_rule_keeps_shift_status_through_and_or_chain() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = r#"bash -c 'unknown && set --; shift 1 && true || git push "$1"' _ safe --force"#;
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
fn force_push_rule_keeps_possible_positional_argv_sets_separate() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for (command, expected) in [
        (
            r#"bash -c 'unknown && set -- echo safe --force; "$@"' _ git push safe"#,
            EvaluationVerdict::Allow,
        ),
        (
            r#"bash -c 'unknown && set -- git push --force; "$@"' _ echo safe"#,
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
fn force_push_rule_bounds_possible_positionals_without_dropping_critical_variants() {
    let current = vec!["echo".to_string(), "safe".to_string()];
    let mut possible = (0..300)
        .map(|index| vec!["echo".to_string(), format!("safe{index}")])
        .collect::<Vec<_>>();
    let critical = vec!["git".to_string(), "push".to_string(), "--force".to_string()];
    possible.push(critical.clone());

    super::super::bash_ast::bound_possible_positional_arguments(&current, &mut possible);

    assert_eq!(
        possible.len(),
        super::super::bash_ast::MAX_STATIC_WORD_VARIANTS - 1
    );
    assert!(possible.contains(&critical));
}
