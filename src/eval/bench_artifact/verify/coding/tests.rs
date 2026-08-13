use anyhow::Result;
use serde_json::Value;

use crate::eval::bench_artifact::tests::{copy_public_fixture, failure_text, mutate_json};
use crate::eval::bench_artifact::types::BenchVerifyOptions;

use super::super::verify_benchmark_artifacts;

#[test]
fn verifier_requires_attempt_state_for_official_coding_runs() -> Result<()> {
    let root = copy_public_fixture("official-attempt-state-missing")?;
    mutate_json(&root.join("coding/reports/coding-report-v1.json"), |json| {
        json["run_phase"] = Value::String("official".to_string());
        json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
    })?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["run_phase"] = Value::String("official".to_string());
            json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("official coding run must include explicit attempt_id"));
    assert!(text.contains("official coding run must include explicit target_started"));
    Ok(())
}

#[test]
fn verifier_rejects_resolved_run_that_never_started_target() -> Result<()> {
    let root = copy_public_fixture("resolved-before-target-start")?;
    mutate_json(&root.join("coding/reports/coding-report-v1.json"), |json| {
        json["run_phase"] = Value::String("official".to_string());
        json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
    })?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["run_phase"] = Value::String("official".to_string());
            json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
            json["attempt_id"] = Value::String("official-attempt-001".to_string());
            json["target_started"] = Value::Bool(false);
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(
        failure_text(&report).contains("resolved coding run cannot report target_started=false")
    );
    Ok(())
}
