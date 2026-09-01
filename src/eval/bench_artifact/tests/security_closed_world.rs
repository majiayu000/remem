use anyhow::{Context, Result};
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;

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
