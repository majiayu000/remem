use anyhow::Result;

use super::*;

fn metrics() -> CodingBenchRunMetrics {
    CodingBenchRunMetrics {
        tokens_input: Some(100),
        tokens_output: Some(25),
        tokens_total: Some(125),
        turns: Some(3),
        wall_time_ms: Some(4_000),
    }
}

fn remem_run(snapshot: RememContractSnapshot) -> CodingBenchRunReport {
    CodingBenchRunReport {
        condition: CodingBenchCondition::Remem,
        task_id: "fixture-task".to_string(),
        run_index: 0,
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
        metrics: metrics(),
        remem_contract_snapshot: Some(snapshot),
    }
}

fn report_with_run(run: CodingBenchRunReport) -> CodingBenchReport {
    CodingBenchReport {
        schema_version: 1,
        benchmark_spec_path: CODING_AGENT_AB_SPEC_PATH,
        current_memory_contract_spec_path: CURRENT_MEMORY_CONTRACT_SPEC_PATH,
        conditions: vec![CodingBenchConditionReport {
            name: run.condition,
            runs: vec![run],
        }],
    }
}

#[test]
fn remem_run_artifact_includes_current_memory_contract_snapshot() -> Result<()> {
    let contract_report =
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval()?;
    let snapshot = build_remem_contract_snapshot(contract_report, 1_800_000_000);
    let report = report_with_run(remem_run(snapshot));

    validate_contract_snapshots(&report)?;
    let json = serde_json::to_value(&report)?;
    let run = &json["conditions"][0]["runs"][0];

    assert_eq!(run["task_success"], true);
    assert_eq!(run["metrics"]["tokens_total"], 125);
    assert_eq!(run["metrics"]["turns"], 3);
    assert_eq!(run["metrics"]["wall_time_ms"], 4_000);
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
    let report = report_with_run(remem_run(snapshot));

    validate_contract_snapshots(&report)?;
    let json = serde_json::to_value(&report)?;
    let run = &json["conditions"][0]["runs"][0];

    assert_eq!(run["task_success"], true);
    assert_eq!(run["task_failure_reason"], serde_json::Value::Null);
    assert_eq!(run["memory_contract_status"], "failed");
    assert_eq!(run["runtime_contract_failure"], true);
    assert!(run["runtime_contract_failure_reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("usage.citation_event_matched")));
    Ok(())
}

#[test]
fn validator_requires_snapshots_only_for_remem_runs() {
    let missing_snapshot = report_with_run(CodingBenchRunReport {
        condition: CodingBenchCondition::Remem,
        task_id: "missing-snapshot".to_string(),
        run_index: 0,
        task_success: true,
        task_failure_reason: None,
        memory_contract_status: CodingBenchMemoryContractStatus::Passed,
        runtime_contract_failure: false,
        runtime_contract_failure_reason: None,
        metrics: metrics(),
        remem_contract_snapshot: None,
    });
    assert!(validate_contract_snapshots(&missing_snapshot)
        .unwrap_err()
        .to_string()
        .contains("missing current memory contract snapshot"));

    let snapshot = build_remem_contract_snapshot(
        crate::eval::current_memory_contracts::run_current_memory_contracts_eval().unwrap(),
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
        metrics: metrics(),
        remem_contract_snapshot: Some(snapshot),
    });
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
        metrics: metrics(),
        remem_contract_snapshot: None,
    });
    assert!(validate_contract_snapshots(&non_remem_contract_failure)
        .unwrap_err()
        .to_string()
        .contains("must not report remem runtime contract failure"));
}
