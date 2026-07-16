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
