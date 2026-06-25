use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{verify_benchmark_artifacts, BenchVerifyOptions};

#[test]
fn committed_public_fixture_passes() -> Result<()> {
    let report = verify_benchmark_artifacts(BenchVerifyOptions {
        root: PathBuf::from("eval/public"),
    })?;

    assert!(report.passed, "{:#?}", report.failures);
    assert_eq!(report.manifests_checked, 4);
    assert_eq!(report.reports_checked, 4);
    assert_eq!(report.run_artifacts_checked, 25);
    assert_eq!(report.artifact_files_checked, 125);
    Ok(())
}

#[test]
fn public_baseline_report_summarizes_committed_artifacts() -> Result<()> {
    let report = super::generate_public_baseline_report(Path::new("eval/public"))?;

    assert!(report.artifact_verifier.passed);
    assert_eq!(report.summary.manifest_count, 4);
    assert_eq!(report.summary.report_count, 4);
    assert_eq!(report.summary.run_artifact_count, 25);
    assert_eq!(report.summary.memory_system.run_artifact_count, 24);
    assert_eq!(report.summary.coding_agent.run_artifact_count, 1);
    assert_eq!(
        report.claim_gate.coding_outcome_stop_loss_status,
        "not_evaluated_insufficient_coding_matrix"
    );
    assert!(report
        .coding_condition_variance
        .iter()
        .any(|entry| entry.variance_status == "insufficient_runs_for_variance"));
    Ok(())
}

#[test]
fn public_baseline_markdown_is_directional_and_separates_layers() -> Result<()> {
    let report = super::generate_public_baseline_report(Path::new("eval/public"))?;
    let markdown = super::render_public_baseline_markdown(&report);

    assert!(markdown.contains("directional_only_no_public_claim"));
    assert!(markdown.contains("## Memory-System Capability"));
    assert!(markdown.contains("## Coding-Agent Outcome"));
    assert!(markdown.contains("insufficient_runs_for_variance"));
    assert!(markdown.contains("must not be used for coding-task superiority claims"));
    Ok(())
}

#[test]
fn verifier_rejects_missing_coding_test_log() -> Result<()> {
    let root = copy_public_fixture("missing-test-log")?;
    fs::remove_file(root.join("coding/artifacts/smoke-coding-001/test.log"))?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("artifact file for test_log is missing"));
    Ok(())
}

#[test]
fn coding_bench_attribution_verifier_rejects_unknown_coding_failure_reason() -> Result<()> {
    let root = copy_public_fixture("unknown-failure-reason")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["resolved"] = Value::Bool(false);
            json["failure_reason"] = Value::String("free_text_failure".to_string());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("unknown failure_reason enum"));
    Ok(())
}

#[test]
fn coding_bench_attribution_verifier_rejects_invalid_memory_contract() -> Result<()> {
    let root = copy_public_fixture("invalid-memory-contract")?;
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["resolved"] = Value::Bool(false);
            json["failure_reason"] = Value::String("stale_memory_followed".to_string());
            json["memory_contract"]["citation_precision"] = Value::from(1.5);
            json["memory_contract"]["memory_hurt"] = Value::Bool(false);
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("memory_contract.citation_precision"));
    assert!(text.contains("memory_contract.memory_hurt=true"));
    Ok(())
}

#[test]
fn verifier_rejects_missing_memory_supporting_ids() -> Result<()> {
    let root = copy_public_fixture("missing-memory-support")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| {
            json["retrieval"]["gold_supporting_event_ids"] = Value::Array(Vec::new());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("missing gold supporting evidence IDs"));
    Ok(())
}

#[test]
fn verifier_rejects_private_remem_data_path() -> Result<()> {
    let root = copy_public_fixture("private-remem-path")?;
    mutate_json(
        &root.join("memory/artifacts/smoke-memory-001/run.json"),
        |json| {
            json["environment"]["remem_data_dir"] = Value::String("~/.remem".to_string());
        },
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    let text = failure_text(&report);
    assert!(text.contains("private remem path"));
    assert!(text.contains("temporary isolation"));
    Ok(())
}

fn failure_text(report: &super::types::BenchVerifyReport) -> String {
    report
        .failures
        .iter()
        .map(|failure| format!("{}: {}", failure.path, failure.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn mutate_json(path: &Path, mutate: impl FnOnce(&mut Value)) -> Result<()> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut json: Value =
        serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    mutate(&mut json);
    fs::write(path, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn copy_public_fixture(label: &str) -> Result<PathBuf> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let root = std::env::temp_dir().join(format!("remem-bench-artifact-{label}-{millis}"));
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    copy_dir_all(Path::new("eval/public"), &root)?;
    Ok(root)
}

fn copy_dir_all(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let from_path = entry.path();
        let to_path = to.join(entry.file_name());
        if from_path.is_dir() {
            copy_dir_all(&from_path, &to_path)?;
        } else {
            fs::copy(&from_path, &to_path).with_context(|| {
                format!("copy {} to {}", from_path.display(), to_path.display())
            })?;
        }
    }
    Ok(())
}
