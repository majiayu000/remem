use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;

use super::{copy_public_fixture, failure_text, mutate_json};
use crate::eval::bench_artifact::{verify_benchmark_artifacts, BenchVerifyOptions};

#[test]
fn verifier_rejects_placeholder_security_snapshot() -> Result<()> {
    let root = copy_public_fixture("placeholder-security-snapshot")?;
    let snapshot = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/remem.db.snapshot.sqlite3",
    );
    fs::write(&snapshot, b"fixture placeholder\n")?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(report.failures.iter().any(|failure| {
        failure.path.ends_with("remem.db.snapshot.sqlite3")
            && (failure.message.contains("SHA-256 mismatch")
                || failure.message.contains("open security SQLite snapshot"))
    }));
    Ok(())
}

#[test]
fn verifier_rejects_hash_valid_snapshot_with_mutated_security_semantics() -> Result<()> {
    let root = copy_public_fixture("mutated-security-semantics")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let snapshot_relative = run["artifacts"]["remem_db_snapshot"]
        .as_str()
        .context("security run must name its snapshot")?;
    let snapshot = root.join(snapshot_relative);
    let connection = rusqlite::Connection::open(&snapshot)?;
    connection.execute(
        "UPDATE captured_events
         SET content_text = 'reviewer-mutated-payload',
             retention_class = 'raw_compact'
         WHERE session_id = 'secrets-api-key-001'",
        [],
    )?;
    drop(connection);
    let mutated_sha256 = format!("{:x}", Sha256::digest(fs::read(&snapshot)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["remem_db_snapshot"] = Value::String(mutated_sha256);
    })?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("snapshot semantic contract"));
    Ok(())
}

#[test]
fn verifier_rejects_snapshot_with_unrelated_captured_event() -> Result<()> {
    let root = copy_public_fixture("unrelated-security-event")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let snapshot_relative = run["artifacts"]["remem_db_snapshot"]
        .as_str()
        .context("security run must name its snapshot")?;
    let snapshot = root.join(snapshot_relative);
    let connection = rusqlite::Connection::open(&snapshot)?;
    let leaked = "private prompt from another session";
    connection.execute(
        "INSERT INTO captured_events (
             host_id, workspace_id, project_id, session_row_id, session_id,
             turn_id, event_id, event_type, role, tool_name, content_text,
             content_blob_id, content_hash, token_estimate, retention_class,
             created_at_epoch, inserted_at_epoch, reference_time_epoch
         )
         SELECT host_id, workspace_id, project_id, session_row_id, 'unrelated-session',
                turn_id, 'unrelated:event', event_type, role, tool_name, ?1,
                NULL, ?2, token_estimate, retention_class,
                created_at_epoch, inserted_at_epoch, reference_time_epoch
         FROM captured_events
         WHERE session_id = 'secrets-api-key-001'
         LIMIT 1",
        rusqlite::params![leaked, crate::db::content_identity_hash(leaked.as_bytes())],
    )?;
    drop(connection);
    let mutated_sha256 = format!("{:x}", Sha256::digest(fs::read(&snapshot)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["remem_db_snapshot"] = Value::String(mutated_sha256);
    })?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions { root })?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("closed-world snapshot inventory"));
    Ok(())
}

#[test]
fn baseline_consumes_the_exact_typed_bytes_verified_before_replacement() -> Result<()> {
    let root = copy_public_fixture("verified-bytes-snapshot")?;
    let verified = verify_benchmark_artifacts(BenchVerifyOptions { root: root.clone() })?;
    let expected = verified
        .verified_artifacts
        .reports
        .iter()
        .find(|artifact| artifact.path == "coding/reports/coding-report-v1.json")
        .context("verified coding report")?
        .value
        .aggregate_metrics
        .clone();
    mutate_json(&root.join("coding/reports/coding-report-v1.json"), |json| {
        json["aggregate_metrics"] = serde_json::json!({"injected_after_verify": true});
    })?;

    let baseline =
        super::super::report::generate_public_baseline_report_from_verified(&root, verified)?;
    let coding = baseline
        .reports
        .iter()
        .find(|report| report.path == "coding/reports/coding-report-v1.json")
        .context("baseline coding report")?;

    assert_eq!(coding.aggregate_metrics, expected);
    Ok(())
}
