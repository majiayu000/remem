use super::*;

#[test]
fn git_rules_accept_path_qualified_executables() {
    let force_artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "/usr/bin/git push --force",
        "command /opt/homebrew/bin/git push --mirror origin",
        r"C:\\Program\ Files\\Git\\cmd\\git.exe push --force",
    ] {
        let outcome = evaluate_artifact(
            &force_artifact,
            &EvaluationInput {
                command: command.to_string(),
            },
        );
        assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{command}");
        assert_eq!(outcome.matches.len(), 1, "{command}");
        assert!(outcome.diagnostics.is_empty(), "{command}");
    }

    let trailer_artifact = CompiledRulesArtifact::new(99, vec![forbidden_trailer_rule()]);
    let outcome = evaluate_artifact(
        &trailer_artifact,
        &EvaluationInput {
            command: "/usr/local/bin/git commit --trailer 'AI-generated-by: Codex' -m safe"
                .to_string(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn force_push_rule_models_mirror_abbreviations_and_boolean_negation() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "git push --m origin",
        "git push --mi origin",
        "git push --mirror --no-mirror --force origin main",
        "git push --force --no-force --mirror origin",
        "git push --no-force -f origin main",
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
        "git push --force --no-force origin main",
        "git push --mirror --no-mirror origin main",
        "git push --mi --no-m origin main",
        "git push -f --no-force origin main",
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
fn force_push_rule_skips_only_statically_unreachable_boolean_branches() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "false && git push --force",
        "true || git push --force",
        "! true && git push --force",
        "! false || git push --force",
        "unknown && false && git push --force",
        "unknown || true || git push --force",
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
    for command in [
        "true && git push --force",
        "false || git push --force",
        "unknown && git push --force",
        "unknown || git push --force",
        "true() { false; }; true || git push --force",
        "true >/definitely/missing/path || git push --force",
        "readonly FLAG; FLAG=x true || git push --force",
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
fn function_state_obeys_unset_and_child_shell_scopes() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "f() { git push --force; }; unset -f f; f",
        "f() { git push --force; }; true && unset -f f; f",
        "f() { git push --force; }; false || unset -f f; f",
        "(f() { git push --force; }); f",
        "echo $(f() { git push --force; }); f",
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
    for command in [
        "f() { git push --force; }; (unset -f f); f",
        "f() { git push --force; }; echo $(unset -f f); f",
        "(f() { git push --force; }; f)",
        "f() { git push --force; }; unknown && unset -f f; f",
        "f() { git push --force; }; unknown && f() { :; }; f",
        "g() { git push --force; }; unknown && f() { unset -f g; }; f; g",
        "g() { git push --force; }; unknown && unset -f g; g",
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
fn exported_functions_are_available_only_to_child_bash() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "f() { git push --force; }; export -f f; bash -c f",
        "f() { git push --force; }; export -f f; bash <<<'f'",
        "f() { git push --force; }; export -f f; unknown && export -nf f; bash -c f",
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
        "f() { git push --force; }; bash -c f",
        "f() { git push --force; }; export -f f; export -nf f; bash -c f",
        "f() { git push --force; }; export -f f; sh -c f",
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
fn nested_static_execution_receivers_preserve_shell_semantics() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "env -v git push --force",
        "env -C/tmp git push --force",
        "builtin eval 'git push --force'",
        "trap 'git push --force' EXIT",
        "trap 'git push --force' 0",
        "source /dev/stdin <<'EOF'\ngit push --force\nEOF",
        ". /dev/stdin <<'EOF'\ngit push --force\nEOF",
        "bash +n -c 'git push --force'",
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
        "bash -n -c 'git push --force'",
        "bash -nc 'git push --force'",
        "bash -o noexec -c 'git push --force'",
        "bash -n +n -n -c 'git push --force'",
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
fn static_if_conditions_prune_only_unreachable_bodies() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "if false; then git push --force; fi",
        "if true; then :; else git push --force; fi",
        "if false; then git push --force; elif true; then :; fi",
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
    for command in [
        "if true; then git push --force; fi",
        "if false; then :; else git push --force; fi",
        "if false; then :; elif true; then git push --force; fi",
        "if unknown; then git push --force; fi",
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
fn shell_stdin_uses_the_effective_final_fd_zero_redirect() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "sh <<'EOF' </dev/null\ngit push --force\nEOF",
        "sh <<< 'git push --force' </dev/null",
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
    for command in [
        "sh </dev/null <<'EOF'\ngit push --force\nEOF",
        "sh 3<<'EOF' <&3\ngit push --force\nEOF",
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
fn env_split_string_preserves_argv_instead_of_shell_syntax() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "env -S 'echo safe; git push --force'",
        "env -S 'printf \"%s\" \"git push --force\"'",
        "env -S 'echo # git push --force'",
        "env FOO=1 -S 'git push --force'",
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
    for command in [
        "env -S 'git push --force'",
        "env -S '-i git push --force'",
        "env -S 'sh -c \"git push --force\"'",
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
fn single_word_brace_expansion_respects_the_materialization_bound() {
    let alternatives = (0..=300)
        .map(|value| format!("item{value}"))
        .collect::<Vec<_>>()
        .join(",");
    let segments = shell_command_segments(&format!("echo {{{alternatives}}}"))
        .expect("large static brace word should remain evaluable");
    assert!(
        segments.len() <= 2,
        "bounded word must not materialize every alternative: {}",
        segments.len()
    );
    let cartesian = shell_command_segments("echo {1..256} {left,right}")
        .expect("capped Cartesian word should remain evaluable");
    assert!(
        cartesian.len() <= 256,
        "bounded Cartesian summary must remain capped: {}",
        cartesian.len()
    );

    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: format!("git push --{{force,{alternatives}}}"),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());

    let semantic_alternatives = ["-vf".to_string(), "--mi".to_string()]
        .into_iter()
        .chain((0..=300).map(|value| format!("branch{value}")))
        .collect::<Vec<_>>()
        .join(",");
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: format!("git push {{{semantic_alternatives}}}"),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());

    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: format!("git push -{{vf,{alternatives}}}"),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());
}
