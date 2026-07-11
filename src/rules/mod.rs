mod artifact;
mod compiler;
mod evaluator;
mod store;

pub use artifact::{
    CompiledRule, CompiledRulesArtifact, RuleAction, RuleOverrideState, RulePredicate,
    ARTIFACT_VERSION,
};
pub use compiler::{
    classify_preference_predicate, classify_preference_predicates, compile_project_rules,
    run_compile_rules_job, CompileOutcome, PreferenceClassification, PreferencePredicate,
};
pub use evaluator::{
    evaluate_artifact, evaluate_artifact_file, EvaluationDiagnostic, EvaluationInput,
    EvaluationOutcome, EvaluationVerdict, RuleMatch,
};
pub use store::{
    artifact_path_for_project, load_artifact_fail_open, write_artifact_atomic, ArtifactLoad,
    ArtifactLoadErrorKind,
};

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;

    use super::{
        CompiledRule, CompiledRulesArtifact, RuleAction, RuleOverrideState, RulePredicate,
    };

    pub(crate) fn package_manager_rule(action: RuleAction) -> CompiledRule {
        CompiledRule {
            rule_id: "pref-123-1".to_string(),
            source_memory_id: 123,
            reinforcement_count: 3,
            action,
            override_state: RuleOverrideState {
                disabled: false,
                action_override: None,
            },
            predicate: RulePredicate::CommandRegex {
                pattern: r"(^|\s)npm\s+(install|i|add)\b".to_string(),
                message: "Command violates a compiled package-manager preference".to_string(),
            },
        }
    }

    pub(crate) fn package_manager_artifact() -> CompiledRulesArtifact {
        CompiledRulesArtifact::new(123, vec![package_manager_rule(RuleAction::Warn)])
    }

    pub(crate) fn test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remem-rules-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
