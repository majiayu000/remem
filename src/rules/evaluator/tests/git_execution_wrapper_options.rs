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
