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
        "git push --{force,no-force} origin main",
        "git push --delete origin +main",
        "git push -d origin +main",
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
        "git() { :; }; git push --force",
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
        "f() { git push --force; }; unset -f f | cat; f",
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
        "bash -s -- arg <<'EOF'\ngit push --force\nEOF",
        "bash -c sh <<'EOF'\ngit push --force\nEOF",
        "env --default-signal=PIPE git push --force",
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

#[test]
fn force_push_rule_tracks_static_shell_state_and_indirect_execution() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "f(){ git push --force;}; declare -fx f; bash -c f",
        "source /dev/fd/0 <<'EOF'\ngit push --force\nEOF",
        "source /proc/self/fd/0 <<'EOF'\ngit push --force\nEOF",
        "builtin builtin eval 'git push --force'",
        "GIT=git env -S '${GIT} push --force'",
        "shopt -s expand_aliases\nalias gp='git push --force'\ngp",
        "git -c alias.pf='push --force' pf",
        "git -c alias.pf='!git push' pf --force",
        "git push -od origin +HEAD:main",
        "shopt -s lastpipe; printf x | { f(){ git push --force;};}; f",
        "git(){ command git \"$@\";}; git push --force",
        "trap 'git push --force' EXIT; case ok in o*) :;; ok) trap - EXIT;; esac",
        "case ok in o*) : ;& nope) git push --force;; esac",
        "f(){ command git \"$@\";}; f push --force",
        "git(){ command git \"$1\" \"$2\";}; git push --force",
        "f(){ command git $@;}; f 'push --force'",
        "f(){ command git $1;}; f 'push --force'",
        "f(){ command git \"${1:-push}\" \"${2:---force}\";}; f",
        "f(){ command \"g$1\" push --force;}; f it",
        "f(){ command \"g${1:-it}\" push --force;}; f",
        "f(){ command git \"${1:-$2}\" \"${3:---force}\";}; f '' push",
        "f(){ command \"${1:-${2:-git}}\" push --force;}; f",
        "git -c $'alias.pf=push\n--force' pf",
        "git -c 'alias.pf=push # --force' pf",
        "bash -c 'sh 3<&0 <&3' <<'EOF'\ngit push --force\nEOF",
        "trap 'git push --force' EXIT; trap -p EXIT",
        "trap 'git push --force' EXIT; exit; trap - EXIT",
        "shopt -s nocasematch; case OK in ok) git push --force;; esac",
        "f(){ git push --force;}; typeset -fx f; bash -c f",
        "shopt -s lastpipe; unknown && set -m; printf x | { f(){ git push --force;};}; f",
        "shopt -s lastpipe; set -m; unknown && set +m; printf x | { f(){ git push --force;};}; f",
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
fn force_push_rule_prunes_static_non_execution_and_honors_overrides() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "while false; do git push --force; done",
        "until true; do git push --force; done",
        "case ok in nope) git push --force;; esac",
        "trap 'git push --force' EXIT; trap - EXIT",
        "f(){ git push --force;}; declare -f f; bash -c f",
        "printf x | { f(){ git push --force;};}; f",
        "bash -c 'sh </dev/null' <<'EOF'\ngit push --force\nEOF",
        "f(){ printf '%s\\n' \"$@\";}; f git push --force",
        "f(){ printf '%s\\n' $@;}; f 'git push --force'",
        "f(){ command git $10 --force;}; f x a b c d e f g h push",
        "f(){ command \"g$1\" push --force;}; f 'it extra'",
        "f(){ command git \"${1:-$2}\" \"${3:---force}\";}; f '' 'push extra'",
        "f(){ command \"${1:-${2:-git}}\" push --force;}; f echo",
        "git -c alias.pf='echo safe; push --force' pf",
        "shopt -s expand_aliases\nalias gp='git push --force'; gp",
        "shopt -s lastpipe; set -m; printf x | { f(){ git push --force;};}; f",
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

    let alternatives = std::iter::once("--force".to_string())
        .chain((0..=300).map(|value| format!("branch{value}")))
        .chain(std::iter::once("--no-force".to_string()))
        .collect::<Vec<_>>()
        .join(",");
    let command = format!("git push {{{alternatives}}}");
    let outcome = evaluate_artifact(&artifact, &EvaluationInput { command });
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow);
    assert!(outcome.matches.is_empty());
    assert!(outcome.diagnostics.is_empty());

    let reverse = std::iter::once("--no-force".to_string())
        .chain((0..=300).map(|value| format!("branch{value}")))
        .chain(std::iter::once("--force".to_string()))
        .collect::<Vec<_>>()
        .join(",");
    let command = format!("git push {{{reverse}}}");
    let outcome = evaluate_artifact(&artifact, &EvaluationInput { command });
    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());

    let alternating = (0..=300)
        .map(|index| {
            if index % 2 == 0 {
                "--force"
            } else {
                "--no-force"
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let command = format!("git push {{{alternating}}}");
    let outcome = evaluate_artifact(&artifact, &EvaluationInput { command });
    assert_eq!(outcome.verdict, EvaluationVerdict::Block);
    assert_eq!(outcome.matches.len(), 1);
    assert!(outcome.diagnostics.is_empty());
}

#[test]
fn force_push_rule_recognizes_exe_shell_basenames() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "bash.exe -c 'git push --force'",
        "/usr/bin/bash.exe -c 'git push --force'",
        r#""C:\Program Files\Git\bin\bash.exe" -c 'git push --force'"#,
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

    let command = "notbash.exe -c 'git push --force'";
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
fn force_push_rule_binds_shell_command_positional_parameters() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    for command in [
        "bash -c 'git push \"$1\"' _ --force",
        "bash -c '\"$0\" push --force' git",
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
        "bash -c 'git push \"$1\"' _ origin",
        "bash -c 'git push \"$1\"' _",
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
fn force_push_rule_preserves_missing_shell_zero() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c '${0:-git} push --force'";
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
fn force_push_rule_keeps_function_positional_scope_inside_shell_command() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let command = "bash -c 'f(){ git push \"$1\"; }; f origin' _ --force";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: command.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Allow, "{command}");
    assert!(outcome.matches.is_empty(), "{command}");
    assert!(outcome.diagnostics.is_empty(), "{command}");

    let command = "bash -c 'f(){ git push \"$1\"; }; f --force' _ origin";
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
fn force_push_rule_resolves_unset_function_before_builtin_state() {
    let artifact = CompiledRulesArtifact::new(99, vec![forbidden_force_push_rule()]);
    let shadowed = "f(){ git push --force;}; unset(){ :;}; unset -f f; f";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: shadowed.into(),
        },
    );
    assert_eq!(outcome.verdict, EvaluationVerdict::Block, "{shadowed}");
    assert!(outcome.diagnostics.is_empty(), "{shadowed}");

    let explicit_builtin = "f(){ git push --force;}; unset(){ :;}; builtin unset -f f; f";
    let outcome = evaluate_artifact(
        &artifact,
        &EvaluationInput {
            command: explicit_builtin.into(),
        },
    );
    assert_eq!(
        outcome.verdict,
        EvaluationVerdict::Allow,
        "{explicit_builtin}"
    );
    assert!(outcome.matches.is_empty(), "{explicit_builtin}");
    assert!(outcome.diagnostics.is_empty(), "{explicit_builtin}");
}

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
