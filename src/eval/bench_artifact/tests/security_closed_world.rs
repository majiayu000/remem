use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::fs::OpenOptions;

use super::{copy_public_fixture, failure_text, mutate_json};
use crate::eval::bench_artifact::{verify_benchmark_artifacts, BenchVerifyOptions};

const SECURITY_RUN: &str = "memory/artifacts/adversarial-policy-v2/\
    remem_default-instruction-injection-001/run.json";
const RELATED_RUN: &str = "memory/artifacts/adversarial-policy-v2/\
    remem_default-approved-external-source-001/run.json";

#[test]
fn verifier_rejects_independent_observation_fts_content() -> Result<()> {
    assert_snapshot_attack_rejected("fts-shadow-injection", SECURITY_RUN, |connection| {
        connection.execute(
            "INSERT INTO observations_fts(rowid, title, narrative)
             VALUES (999999, 'reviewer private index', 'private secret payload')",
            [],
        )?;
        Ok(())
    })
}

#[test]
fn verifier_rejects_unknown_business_column_with_private_content() -> Result<()> {
    assert_snapshot_attack_rejected("business-schema-injection", SECURITY_RUN, |connection| {
        connection.execute_batch(
            "ALTER TABLE captured_events
                 ADD COLUMN reviewer_private_payload TEXT;
             UPDATE captured_events
                SET reviewer_private_payload = 'private payload outside the typed event';",
        )?;
        Ok(())
    })
}

#[test]
fn verifier_rejects_mutated_related_business_row() -> Result<()> {
    assert_snapshot_attack_rejected("related-row-injection", RELATED_RUN, |connection| {
        connection.execute(
            "UPDATE entities
                SET canonical_name = 'private customer secret from another task'",
            [],
        )?;
        Ok(())
    })
}

#[test]
fn verifier_rejects_unreferenced_files_in_security_run_directory() -> Result<()> {
    let root = copy_public_fixture("unreferenced-security-artifact")?;
    let run_path = root.join(SECURITY_RUN);
    fs::write(
        run_path
            .parent()
            .context("security run parent")?
            .join("private.log"),
        "private prompt payload",
    )?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("unreferenced file"));
    Ok(())
}

#[test]
fn verifier_privacy_scans_declared_text_artifact_payloads() -> Result<()> {
    let root = copy_public_fixture("private-declared-artifact")?;
    let run_path = root.join(SECURITY_RUN);
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let answer_relative = run["artifacts"]["answer"]
        .as_str()
        .context("security run answer artifact")?;
    let answer_path = root.join(answer_relative);
    fs::write(
        &answer_path,
        serde_json::to_vec(&serde_json::json!({
            "abstained": true,
            "text": "opened /home/runner/private.txt during verification"
        }))?,
    )?;
    let answer_sha256 = format!("{:x}", Sha256::digest(fs::read(&answer_path)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["answer"] = Value::String(answer_sha256);
    })?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("absolute user home path"));
    Ok(())
}

#[test]
fn verifier_rejects_non_utf8_declared_text_artifacts() -> Result<()> {
    let root = copy_public_fixture("non-utf8-declared-artifact")?;
    let run_path = root.join(SECURITY_RUN);
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let answer_relative = run["artifacts"]["answer"]
        .as_str()
        .context("security run answer artifact")?;
    let answer_path = root.join(answer_relative);
    fs::write(&answer_path, [0xff, 0xfe])?;
    let answer_sha256 = format!("{:x}", Sha256::digest(fs::read(&answer_path)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["answer"] = Value::String(answer_sha256);
    })?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("must be UTF-8"));
    Ok(())
}

#[test]
fn verifier_binds_security_report_suite_identity_to_suite_bytes() -> Result<()> {
    let root = copy_public_fixture("security-report-suite-identity")?;
    mutate_json(
        &root.join("memory/reports/adversarial-policy-v2.json"),
        |report| {
            report["aggregate_metrics"]["suite_content_identity"] =
                Value::String(format!("sha256-raw-suite-v1:{}", "f".repeat(64)));
        },
    )?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("report suite identity"));
    Ok(())
}

#[test]
fn verifier_bounds_declared_non_sqlite_artifacts_before_reading() -> Result<()> {
    let root = copy_public_fixture("oversized-declared-artifact")?;
    let run: Value = serde_json::from_slice(&fs::read(root.join(SECURITY_RUN))?)?;
    let answer_relative = run["artifacts"]["answer"]
        .as_str()
        .context("security run answer artifact")?;
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(root.join(answer_relative))?
        .set_len(64 * 1024 * 1024 + 1)?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("exceeds the 64 MiB public artifact limit"));
    Ok(())
}

#[test]
fn security_identity_failure_makes_top_level_verifier_fail() -> Result<()> {
    let root = copy_public_fixture("security-identity-top-level-failure")?;
    mutate_json(&root.join(SECURITY_RUN), |run| {
        run["reader_model"]["model"] = Value::String("different-model".to_string());
    })?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("model execution identity"));
    Ok(())
}

fn assert_snapshot_attack_rejected(
    label: &str,
    run_relative: &str,
    attack: impl FnOnce(&Connection) -> Result<()>,
) -> Result<()> {
    let root = copy_public_fixture(label)?;
    let run_path = root.join(run_relative);
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let snapshot_relative = run["artifacts"]["remem_db_snapshot"]
        .as_str()
        .context("security run must name its snapshot")?;
    let snapshot = root.join(snapshot_relative);
    let connection = Connection::open(&snapshot)?;
    attack(&connection)?;
    connection.execute_batch("VACUUM")?;
    drop(connection);
    let mutated_sha256 = format!("{:x}", Sha256::digest(fs::read(&snapshot)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["remem_db_snapshot"] = Value::String(mutated_sha256);
    })?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed, "reviewer attack {label} was accepted");
    assert!(failure_text(&report).contains("snapshot semantic contract"));
    Ok(())
}
