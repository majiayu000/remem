use super::score::{used_irrelevant_memory, used_unknown_memory};
use super::types::{
    CodingBenchFailureReason, CodingMemoryAttribution, CodingMemoryAttributionInput,
};

pub(crate) struct FailureEvidence<'a> {
    pub score_failed: bool,
    pub compile_failed: bool,
    pub forbidden_patch_failed: bool,
    pub unauthorized_paths: &'a [String],
    pub memory_contract: Option<&'a CodingMemoryAttribution>,
    pub memory_input: &'a CodingMemoryAttributionInput,
    pub runner_isolation_violation: Option<&'a str>,
    pub runner_timed_out: bool,
    pub runner_exit_code: Option<i32>,
    pub runner_stdout: &'a str,
    pub runner_stderr: &'a str,
}

pub(crate) fn classify_failure_reason(
    evidence: FailureEvidence<'_>,
) -> Option<CodingBenchFailureReason> {
    if evidence.runner_isolation_violation.is_some() {
        return Some(CodingBenchFailureReason::OracleInconclusive);
    }
    if evidence.runner_timed_out {
        return Some(CodingBenchFailureReason::Timeout);
    }
    if output_indicates_over_context_budget(evidence.runner_stdout, evidence.runner_stderr) {
        return Some(CodingBenchFailureReason::OverContextBudget);
    }
    if !evidence.unauthorized_paths.is_empty() {
        return Some(CodingBenchFailureReason::WrongFileModified);
    }
    if evidence.score_failed {
        if let Some(attribution) = evidence.memory_contract {
            if used_unknown_memory(attribution) {
                return Some(CodingBenchFailureReason::AgentHallucinatedMemory);
            }
            if attribution.stale_used_count > 0
                || (evidence.forbidden_patch_failed
                    && !evidence.memory_input.gold_forbidden_facts.is_empty())
            {
                return Some(CodingBenchFailureReason::StaleMemoryFollowed);
            }
            if attribution.missing_relevant_memory_count > 0 {
                return Some(CodingBenchFailureReason::MissingMemory);
            }
            if !evidence.memory_input.gold_required_facts.is_empty()
                && evidence.memory_input.relevant_memory_ids.is_empty()
            {
                return Some(CodingBenchFailureReason::MissingMemory);
            }
            if !evidence.memory_input.relevant_memory_ids.is_empty()
                && attribution
                    .used_memory_ids
                    .iter()
                    .all(|id| !evidence.memory_input.relevant_memory_ids.contains(id))
            {
                return Some(CodingBenchFailureReason::IgnoredMemory);
            }
            if used_irrelevant_memory(attribution, evidence.memory_input) {
                return Some(CodingBenchFailureReason::IrrelevantMemoryDistracted);
            }
        }
        if evidence.compile_failed {
            return Some(CodingBenchFailureReason::CompileFailure);
        }
        return Some(CodingBenchFailureReason::TestFailure);
    }
    if evidence.runner_exit_code != Some(0) {
        return Some(CodingBenchFailureReason::OracleInconclusive);
    }
    None
}

pub(crate) fn output_indicates_compile_failure(stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    text.contains("could not compile")
        || text.contains("compilation failed")
        || text.contains("error[e")
        || text.contains("syntaxerror")
        || text.contains("compileerror")
}

fn output_indicates_over_context_budget(stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    (text.contains("context") || text.contains("token"))
        && (text.contains("too large")
            || text.contains("maximum context")
            || text.contains("context length")
            || text.contains("over context")
            || text.contains("exceeds"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure_evidence<'a>(
        memory_contract: Option<&'a CodingMemoryAttribution>,
        memory_input: &'a CodingMemoryAttributionInput,
    ) -> FailureEvidence<'a> {
        FailureEvidence {
            score_failed: true,
            compile_failed: false,
            forbidden_patch_failed: false,
            unauthorized_paths: &[],
            memory_contract,
            memory_input,
            runner_isolation_violation: None,
            runner_timed_out: false,
            runner_exit_code: Some(0),
            runner_stdout: "",
            runner_stderr: "",
        }
    }

    #[test]
    fn coding_bench_attribution_classifies_stale_memory_before_generic_test_failure() {
        let attribution = CodingMemoryAttribution {
            injected_memory_ids: vec![7],
            used_memory_ids: vec![7],
            citation_precision: 1.0,
            citation_recall: 1.0,
            stale_used_count: 1,
            irrelevant_injection_count: 0,
            missing_relevant_memory_count: 0,
            memory_helped: false,
            memory_hurt: true,
        };
        let input = CodingMemoryAttributionInput {
            injected_memory_ids: vec![7],
            relevant_memory_ids: Vec::new(),
            forbidden_memory_ids: vec![7],
            gold_required_facts: Vec::new(),
            gold_forbidden_facts: vec!["fact:old_api".to_string()],
        };

        assert_eq!(
            classify_failure_reason(failure_evidence(Some(&attribution), &input)),
            Some(CodingBenchFailureReason::StaleMemoryFollowed)
        );
    }

    #[test]
    fn output_compile_classifier_detects_rust_compile_errors() {
        assert!(output_indicates_compile_failure(
            "",
            "error[E0308]: mismatched types\ncould not compile `fixture`"
        ));
    }
}
