use anyhow::Result;

use super::*;

const TEST_RUNS_PER_CONDITION: usize = MIN_RUNS_PER_CONDITION;
const TEST_FINAL_HEAD_SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn score_evidence() -> CodingBenchRunScoreEvidence {
    CodingBenchRunScoreEvidence {
        commands: vec![CodingBenchScoreCommandEvidence {
            command: vec![
                "cargo".to_string(),
                "test".to_string(),
                "--test".to_string(),
                "fixture".to_string(),
            ],
            exit_code: 0,
            stdout: Some("test result: ok".to_string()),
            stderr: None,
            output_artifact_path: None,
        }],
    }
}

fn metrics() -> CodingBenchRunMetrics {
    CodingBenchRunMetrics {
        tokens_input: Some(100),
        tokens_output: Some(25),
        tokens_total: Some(125),
        token_accounting_unsupported_reason: None,
        turns: Some(3),
        wall_time_ms: Some(4_000),
    }
}

fn remem_run(snapshot: RememContractSnapshot) -> CodingBenchRunReport {
    remem_run_for_task(snapshot, "fixture-task", 0)
}

fn remem_run_for_task(
    snapshot: RememContractSnapshot,
    task_id: &str,
    run_index: usize,
) -> CodingBenchRunReport {
    CodingBenchRunReport {
        condition: CodingBenchCondition::Remem,
        task_id: task_id.to_string(),
        run_index,
        task_success: true,
        task_failure_reason: None,
        memory_contract_status: if snapshot.contract_health.all_checks_passed {
            CodingBenchMemoryContractStatus::Passed
        } else {
            CodingBenchMemoryContractStatus::Failed
        },
        runtime_contract_failure: !snapshot.contract_health.all_checks_passed,
        runtime_contract_failure_reason: (!snapshot.contract_health.all_checks_passed)
            .then(|| snapshot.contract_health.failing_examples.join("; ")),
        score: score_evidence(),
        metrics: metrics(),
        final_head_sha: Some(TEST_FINAL_HEAD_SHA.to_string()),
        patch_artifact_path: None,
        unauthorized_path_changes: Vec::new(),
        remem_contract_snapshot: Some(snapshot),
    }
}

fn control_run(
    condition: CodingBenchCondition,
    task_id: &str,
    run_index: usize,
) -> CodingBenchRunReport {
    CodingBenchRunReport {
        condition,
        task_id: task_id.to_string(),
        run_index,
        task_success: true,
        task_failure_reason: None,
        memory_contract_status: CodingBenchMemoryContractStatus::NotApplicable,
        runtime_contract_failure: false,
        runtime_contract_failure_reason: None,
        score: score_evidence(),
        metrics: metrics(),
        final_head_sha: Some(TEST_FINAL_HEAD_SHA.to_string()),
        patch_artifact_path: None,
        unauthorized_path_changes: Vec::new(),
        remem_contract_snapshot: None,
    }
}

fn condition_report(
    name: CodingBenchCondition,
    task_id: &str,
) -> Result<CodingBenchConditionReport> {
    let runs = match name {
        CodingBenchCondition::Remem => {
            let snapshot = build_remem_contract_snapshot(
                crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?,
                1_800_000_000,
            );
            (0..TEST_RUNS_PER_CONDITION)
                .map(|run_index| remem_run_for_task(snapshot.clone(), task_id, run_index))
                .collect()
        }
        CodingBenchCondition::NoMemory | CodingBenchCondition::CuratedFile => (0
            ..TEST_RUNS_PER_CONDITION)
            .map(|run_index| control_run(name, task_id, run_index))
            .collect(),
    };
    Ok(CodingBenchConditionReport { name, runs })
}

fn report_with_run(run: CodingBenchRunReport) -> Result<CodingBenchReport> {
    let task_id = run.task_id.clone();
    let mut report = CodingBenchReport {
        schema_version: 1,
        benchmark_spec_path: CODING_AGENT_AB_SPEC_PATH,
        current_memory_contract_spec_path: CURRENT_MEMORY_CONTRACT_SPEC_PATH,
        runs_per_condition: TEST_RUNS_PER_CONDITION,
        conditions: vec![
            condition_report(CodingBenchCondition::Remem, &task_id)?,
            condition_report(CodingBenchCondition::NoMemory, &task_id)?,
            condition_report(CodingBenchCondition::CuratedFile, &task_id)?,
        ],
    };

    let target_condition = report
        .conditions
        .iter_mut()
        .find(|condition| condition.name == run.condition);
    if let Some(condition) = target_condition {
        let run_index = run.run_index;
        if run_index < condition.runs.len() {
            condition.runs[run_index] = run;
        } else {
            condition.runs.push(run);
        }
    }
    Ok(report)
}

#[test]
fn remem_run_artifact_includes_current_memory_contract_snapshot() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let report = report_with_run(remem_run(snapshot))?;

    validate_contract_snapshots(&report)?;
    let json = serde_json::to_value(&report)?;
    let run = &json["conditions"][0]["runs"][0];
    let run_object = run.as_object().expect("run serializes as an object");

    assert_eq!(run["resolved"], true);
    assert!(!run_object.contains_key("task_success"));
    assert_eq!(run["metrics"]["tokens_total"], 125);
    assert_eq!(run["metrics"]["turns"], 3);
    assert_eq!(run["metrics"]["wall_time_ms"], 4_000);
    assert_eq!(run["score"]["commands"][0]["command"][0], "cargo");
    assert_eq!(run["score"]["commands"][0]["stdout"], "test result: ok");
    assert_eq!(run["final_head_sha"], TEST_FINAL_HEAD_SHA);
    assert_eq!(run["patch_artifact_path"], serde_json::Value::Null);
    assert!(run["unauthorized_path_changes"]
        .as_array()
        .is_some_and(|changes| changes.is_empty()));
    assert_eq!(run["memory_contract_status"], "passed");
    assert_eq!(
        run["remem_contract_snapshot"]["contract_health"]["all_checks_passed"],
        true
    );
    assert_eq!(
        run["remem_contract_snapshot"]["citation_precision"]["rate"],
        1.0
    );
    assert_eq!(
        run["remem_contract_snapshot"]["staleness_handling"]["verify_before_trust"]["rate"],
        1.0
    );
    assert_eq!(
        run["remem_contract_snapshot"]["staleness_handling"]["history_tracked"]["rate"],
        1.0
    );
    assert_eq!(
        run["remem_contract_snapshot"]["temporal_fact_eligibility"]["invalidated_fact_exclusion"]
            ["rate"],
        1.0
    );
    assert_eq!(
        run["remem_contract_snapshot"]["injected_memory_audit"]["injected"]["rate"],
        1.0
    );
    assert_eq!(
        run["remem_contract_snapshot"]["usage_feedback_coverage"]
            ["usage_event_linked_to_injection_item"]["rate"],
        1.0
    );
    assert_eq!(
        run["remem_contract_snapshot"]["current_memory_contracts"]["metrics"]["all_checks_passed"],
        true
    );
    Ok(())
}

#[test]
fn runtime_contract_failure_is_distinct_from_agent_task_failure() -> Result<()> {
    let mut contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    contract_report.metrics.all_checks_passed = false;
    contract_report
        .failing_examples
        .push("usage.citation_event_matched expected 1.0 but got 0.0".to_string());
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let report = report_with_run(remem_run(snapshot))?;

    validate_contract_snapshots(&report)?;
    let json = serde_json::to_value(&report)?;
    let run = &json["conditions"][0]["runs"][0];

    assert_eq!(run["resolved"], true);
    assert_eq!(run["failure_reason"], serde_json::Value::Null);
    assert!(run
        .as_object()
        .is_some_and(|object| !object.contains_key("task_failure_reason")));
    assert_eq!(run["memory_contract_status"], "failed");
    assert_eq!(run["runtime_contract_failure"], true);
    assert!(run["runtime_contract_failure_reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("usage.citation_event_matched")));
    Ok(())
}

#[test]
fn validator_rejects_stale_runtime_contract_failure_reason() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let mut run = remem_run(snapshot);
    run.runtime_contract_failure = false;
    run.runtime_contract_failure_reason = Some("stale contract failure".to_string());
    let report = report_with_run(run)?;

    assert!(validate_contract_snapshots(&report)
        .unwrap_err()
        .to_string()
        .contains("stale runtime_contract_failure_reason"));
    Ok(())
}

#[test]
fn validator_rejects_blank_runtime_contract_failure_reason() -> Result<()> {
    let mut contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    contract_report.metrics.all_checks_passed = false;
    contract_report
        .failing_examples
        .push("usage.citation_event_matched expected 1.0 but got 0.0".to_string());
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let mut run = remem_run(snapshot);
    run.runtime_contract_failure_reason = Some(" \n\t ".to_string());
    let report = report_with_run(run)?;

    assert!(validate_contract_snapshots(&report)
        .unwrap_err()
        .to_string()
        .contains("runtime contract failure without reason"));
    Ok(())
}

#[test]
fn validator_rejects_stale_task_failure_reason_on_resolved_run() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let mut run = remem_run(snapshot);
    run.task_success = true;
    run.task_failure_reason = Some("stale task failure".to_string());
    let report = report_with_run(run)?;

    assert!(validate_contract_snapshots(&report)
        .unwrap_err()
        .to_string()
        .contains("stale task_failure_reason"));
    Ok(())
}

#[test]
fn validator_requires_score_and_patch_evidence() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);

    let mut missing_score = remem_run(snapshot.clone());
    missing_score.score.commands.clear();
    assert!(
        validate_contract_snapshots(&report_with_run(missing_score)?)
            .unwrap_err()
            .to_string()
            .contains("missing score command evidence")
    );

    let mut missing_score_output = remem_run(snapshot.clone());
    missing_score_output.score.commands[0].stdout = Some(" ".to_string());
    missing_score_output.score.commands[0].stderr = None;
    missing_score_output.score.commands[0].output_artifact_path = None;
    assert!(
        validate_contract_snapshots(&report_with_run(missing_score_output)?)
            .unwrap_err()
            .to_string()
            .contains("score command 0 has no output evidence")
    );

    let mut patch_only = remem_run(snapshot.clone());
    patch_only.final_head_sha = None;
    patch_only.patch_artifact_path = Some("artifacts/fixture-task-0.patch".to_string());
    validate_contract_snapshots(&report_with_run(patch_only)?)?;

    let mut missing_patch_evidence = remem_run(snapshot.clone());
    missing_patch_evidence.final_head_sha = None;
    missing_patch_evidence.patch_artifact_path = None;
    assert!(
        validate_contract_snapshots(&report_with_run(missing_patch_evidence)?)
            .unwrap_err()
            .to_string()
            .contains("missing final_head_sha or patch_artifact_path")
    );

    let mut blank_unauthorized_path = remem_run(snapshot);
    blank_unauthorized_path
        .unauthorized_path_changes
        .push(" ".to_string());
    assert!(
        validate_contract_snapshots(&report_with_run(blank_unauthorized_path)?)
            .unwrap_err()
            .to_string()
            .contains("blank unauthorized path change")
    );
    Ok(())
}

#[test]
fn validator_requires_token_accounting_or_unsupported_provider_note() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let mut missing_tokens = remem_run(snapshot.clone());
    missing_tokens.metrics.tokens_input = None;
    missing_tokens.metrics.tokens_output = None;
    missing_tokens.metrics.tokens_total = None;

    assert!(
        validate_contract_snapshots(&report_with_run(missing_tokens.clone())?)
            .unwrap_err()
            .to_string()
            .contains("missing token accounting without token_accounting_unsupported_reason")
    );

    let mut unsupported_provider = missing_tokens;
    unsupported_provider
        .metrics
        .token_accounting_unsupported_reason =
        Some("provider does not expose token usage for coding-bench runs".to_string());
    validate_contract_snapshots(&report_with_run(unsupported_provider)?)?;

    let mut partial_tokens = remem_run(snapshot.clone());
    partial_tokens.metrics.tokens_output = None;
    assert!(
        validate_contract_snapshots(&report_with_run(partial_tokens)?)
            .unwrap_err()
            .to_string()
            .contains("complete token accounting")
    );

    let mut mismatched_total = remem_run(snapshot);
    mismatched_total.metrics.tokens_total = Some(126);
    assert!(
        validate_contract_snapshots(&report_with_run(mismatched_total)?)
            .unwrap_err()
            .to_string()
            .contains("tokens_total=126 does not equal tokens_input + tokens_output")
    );
    Ok(())
}

#[test]
fn validator_requires_turns_and_wall_time_metrics() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);

    let mut missing_turns = remem_run(snapshot.clone());
    missing_turns.metrics.turns = None;
    assert!(
        validate_contract_snapshots(&report_with_run(missing_turns)?)
            .unwrap_err()
            .to_string()
            .contains("missing turns")
    );

    let mut missing_wall_time = remem_run(snapshot);
    missing_wall_time.metrics.wall_time_ms = None;
    assert!(
        validate_contract_snapshots(&report_with_run(missing_wall_time)?)
            .unwrap_err()
            .to_string()
            .contains("missing wall_time_ms")
    );
    Ok(())
}

#[test]
fn validator_requires_full_condition_matrix() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let report = report_with_run(remem_run(snapshot))?;

    let mut missing_condition = report.clone();
    missing_condition
        .conditions
        .retain(|condition| condition.name != CodingBenchCondition::NoMemory);
    assert!(validate_contract_snapshots(&missing_condition)
        .unwrap_err()
        .to_string()
        .contains("missing required NoMemory condition"));

    let mut too_few_repeats = report.clone();
    for condition in &mut too_few_repeats.conditions {
        if condition.name == CodingBenchCondition::CuratedFile {
            condition.runs.pop();
        }
    }
    assert!(validate_contract_snapshots(&too_few_repeats)
        .unwrap_err()
        .to_string()
        .contains("CuratedFile condition missing run fixture-task#2"));

    let mut invalid_repeat_contract = report;
    invalid_repeat_contract.runs_per_condition = MIN_RUNS_PER_CONDITION - 1;
    assert!(validate_contract_snapshots(&invalid_repeat_contract)
        .unwrap_err()
        .to_string()
        .contains("below required minimum"));
    Ok(())
}

#[test]
fn validator_derives_contract_health_from_embedded_report() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let mut snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    snapshot.current_memory_contracts.metrics.all_checks_passed = false;
    let report = report_with_run(remem_run(snapshot))?;

    assert!(validate_contract_snapshots(&report)
        .unwrap_err()
        .to_string()
        .contains("contract_health does not match embedded current_memory_contracts"));
    Ok(())
}

#[test]
fn validator_requires_snapshots_only_for_remem_runs() -> Result<()> {
    let missing_snapshot = report_with_run(CodingBenchRunReport {
        condition: CodingBenchCondition::Remem,
        task_id: "missing-snapshot".to_string(),
        run_index: 0,
        task_success: true,
        task_failure_reason: None,
        memory_contract_status: CodingBenchMemoryContractStatus::Passed,
        runtime_contract_failure: false,
        runtime_contract_failure_reason: None,
        score: score_evidence(),
        metrics: metrics(),
        final_head_sha: Some(TEST_FINAL_HEAD_SHA.to_string()),
        patch_artifact_path: None,
        unauthorized_path_changes: Vec::new(),
        remem_contract_snapshot: None,
    })?;
    assert!(validate_contract_snapshots(&missing_snapshot)
        .unwrap_err()
        .to_string()
        .contains("missing current memory contract snapshot"));

    let snapshot = build_remem_contract_snapshot(
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?,
        1_800_000_000,
    );
    let non_remem_snapshot = report_with_run(CodingBenchRunReport {
        condition: CodingBenchCondition::NoMemory,
        task_id: "bad-no-memory".to_string(),
        run_index: 0,
        task_success: true,
        task_failure_reason: None,
        memory_contract_status: CodingBenchMemoryContractStatus::NotApplicable,
        runtime_contract_failure: false,
        runtime_contract_failure_reason: None,
        score: score_evidence(),
        metrics: metrics(),
        final_head_sha: Some(TEST_FINAL_HEAD_SHA.to_string()),
        patch_artifact_path: None,
        unauthorized_path_changes: Vec::new(),
        remem_contract_snapshot: Some(snapshot),
    })?;
    assert!(validate_contract_snapshots(&non_remem_snapshot)
        .unwrap_err()
        .to_string()
        .contains("must not carry a remem contract snapshot"));

    let non_remem_contract_failure = report_with_run(CodingBenchRunReport {
        condition: CodingBenchCondition::CuratedFile,
        task_id: "bad-curated-file".to_string(),
        run_index: 0,
        task_success: true,
        task_failure_reason: None,
        memory_contract_status: CodingBenchMemoryContractStatus::NotApplicable,
        runtime_contract_failure: true,
        runtime_contract_failure_reason: Some("current-memory contract failed".to_string()),
        score: score_evidence(),
        metrics: metrics(),
        final_head_sha: Some(TEST_FINAL_HEAD_SHA.to_string()),
        patch_artifact_path: None,
        unauthorized_path_changes: Vec::new(),
        remem_contract_snapshot: None,
    })?;
    assert!(validate_contract_snapshots(&non_remem_contract_failure)
        .unwrap_err()
        .to_string()
        .contains("must not report remem runtime contract failure"));
    Ok(())
}
