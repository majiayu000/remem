use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::eval::bench_artifact::tests::{copy_public_fixture, failure_text, mutate_json};
use crate::eval::bench_artifact::types::{BenchVerifyOptions, CodingRunArtifact};

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

    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;

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

    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;

    assert!(!report.passed);
    assert!(
        failure_text(&report).contains("resolved coding run cannot report target_started=false")
    );
    Ok(())
}

#[test]
fn official_failing_or_timed_out_commands_cannot_authorize_declared_resolution() -> Result<()> {
    let root = copy_public_fixture("official-failing-test-evidence")?;
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["benchmark_id"] = Value::String("issue385-v1".to_string());
            if json.get("version").is_some() {
                json["version"] = Value::String("official-v1".to_string());
            } else {
                json["benchmark_version"] = Value::String("official-v1".to_string());
                json["run_phase"] = Value::String("official".to_string());
                json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
            }
            json["conditions"] = serde_json::json!(["no_memory"]);
        })?;
    }
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["benchmark_id"] = Value::String("issue385-v1".to_string());
            json["benchmark_version"] = Value::String("official-v1".to_string());
            json["task_id"] = Value::String("ticket-key-memory-convention".to_string());
            json["run_phase"] = Value::String("official".to_string());
            json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
            json["condition"] = Value::String("no_memory".to_string());
            json["attempt_id"] = Value::String("official-attempt-001".to_string());
            json["target_started"] = Value::Bool(true);
            json["context_audit_status"] = Value::String("not_applicable".to_string());
            json["context_audit_failure_reason"] = Value::Null;
            json["remem_context_audit"] = Value::Null;
            json["injected_context_sha256"] = Value::Null;
            json["memory_contract"] = Value::Null;
        },
    )?;
    fs::write(
        root.join("coding/artifacts/smoke-coding-001/test.log"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "task_id": "ticket-key-memory-convention",
            "condition": "no_memory",
            "run_index": 0,
            "attempt_id": "official-attempt-001",
            "commands": [{
                "command": "true",
                "exit_code": 0,
                "timed_out": false
            }]
        }))?,
    )?;

    let forged = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;
    assert!(!forged.passed);
    assert!(failure_text(&forged).contains("registered scorer command"));

    fs::write(
        root.join("coding/artifacts/smoke-coding-001/test.log"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "task_id": "ticket-key-memory-convention",
            "condition": "no_memory",
            "run_index": 0,
            "attempt_id": "official-attempt-001",
            "commands": [{
                "command": "python3 -m unittest tests.test_ticket_hidden",
                "exit_code": 1,
                "timed_out": false
            }]
        }))?,
    )?;

    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains(
        "declared coding resolution/failure_reason disagrees with recomputed test evidence"
    ));
    let recomputed = report
        .verified_artifacts
        .official_coding_tests
        .values()
        .next()
        .expect("verified official test evidence");
    assert!(!recomputed.value.resolved());
    assert_ne!(
        report.authority_verdict.gh931.status,
        crate::eval::bench_artifact::AuthorityStatus::Pass
    );

    fs::write(
        root.join("coding/artifacts/smoke-coding-001/test.log"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "task_id": "ticket-key-memory-convention",
            "condition": "no_memory",
            "run_index": 0,
            "attempt_id": "official-attempt-001",
            "commands": [
                {
                    "command": "python3 -m unittest tests.test_ticket_hidden",
                    "exit_code": null,
                    "timed_out": true
                }
            ]
        }))?,
    )?;
    let timed_out = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;
    assert!(!timed_out.passed);
    assert!(failure_text(&timed_out).contains(
        "declared coding resolution/failure_reason disagrees with recomputed test evidence"
    ));
    assert!(!timed_out
        .verified_artifacts
        .official_coding_tests
        .values()
        .next()
        .expect("timed-out official test evidence")
        .value
        .resolved());

    fs::write(
        root.join("coding/artifacts/smoke-coding-001/test.log"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "task_id": "ticket-key-memory-convention",
            "condition": "no_memory",
            "run_index": 0,
            "attempt_id": "official-attempt-001",
            "commands": []
        }))?,
    )?;
    let empty =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;
    assert!(!empty.passed);
    assert!(failure_text(&empty).contains("official coding test evidence requires commands"));
    Ok(())
}

#[test]
fn official_treatment_maintenance_rejects_inconsistent_zero_work() -> Result<()> {
    let root = copy_public_fixture("official-inconsistent-zero-maintenance")?;
    for path in [
        "coding/manifests/issue385-smoke-v1.json",
        "coding/reports/coding-report-v1.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["benchmark_id"] = Value::String("issue385-v1".to_string());
            if json.get("version").is_some() {
                json["version"] = Value::String("official-v1".to_string());
            } else {
                json["benchmark_version"] = Value::String("official-v1".to_string());
                json["run_phase"] = Value::String("official".to_string());
                json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
            }
            json["conditions"] = serde_json::json!(["remem_e2e"]);
        })?;
    }
    mutate_json(
        &root.join("coding/artifacts/smoke-coding-001/run.json"),
        |json| {
            json["benchmark_id"] = Value::String("issue385-v1".to_string());
            json["benchmark_version"] = Value::String("official-v1".to_string());
            json["run_phase"] = Value::String("official".to_string());
            json["matrix_namespace"] = Value::String("issue385-v1/official-v1".to_string());
            json["condition"] = Value::String("remem_e2e".to_string());
            json["attempt_id"] = Value::String("official-attempt-001".to_string());
            json["target_started"] = Value::Bool(true);
            json["artifacts"]["maintenance_evidence"] =
                Value::String("coding/artifacts/smoke-coding-001/maintenance.json".to_string());
        },
    )?;
    fs::write(
        root.join("coding/artifacts/smoke-coding-001/test.log"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "task_id": "smoke-fix-startup-race-001",
            "condition": "remem_e2e",
            "run_index": 0,
            "attempt_id": "official-attempt-001",
            "commands": [{
                "command": "cargo test --test target",
                "exit_code": 0,
                "timed_out": false
            }]
        }))?,
    )?;
    fs::write(
        root.join("coding/artifacts/smoke-coding-001/maintenance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "task_id": "smoke-fix-startup-race-001",
            "condition": "remem_e2e",
            "run_index": 0,
            "attempt_id": "official-attempt-001",
            "measurement": {
                "kind": "zero_work",
                "minutes": 1.0,
                "work_events": 0,
                "session_count": 0
            }
        }))?,
    )?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report)
        .contains("official remem_e2e maintenance evidence is unbound or internally inconsistent"));
    Ok(())
}

#[test]
fn coding_context_audit_uses_consumed_sqlite_bytes_after_source_removed() -> Result<()> {
    let root = copy_public_fixture("coding-snapshot-consumed-bytes")?;
    let context_path = root.join("coding-context.txt");
    let database_path = root.join("coding-context.sqlite3");
    let injected_context = "test";
    fs::write(&context_path, injected_context)?;

    let connection = Connection::open(&database_path)?;
    crate::migrate::run_migrations(&connection)?;
    let injection_run_id = "coding-consumed-snapshot-run";
    connection.execute(
        "INSERT INTO context_injection_items
         (injection_run_id, host, project, injection_key, context_hash, output_mode,
          decision, item_kind, channel, status, injected_at_epoch)
         VALUES (?1, 'codex-cli', 'project', 'key', ?2, 'full', 'emitted',
                 'sessionstart_relevance_policy', 'policy', 'injected', 100)",
        params![
            injection_run_id,
            crate::context::context_output_fingerprint(injected_context)
        ],
    )?;
    let bundle = empty_context_bundle(1);
    crate::context_bundle::persist_context_bundle_audit(
        &connection,
        injection_run_id,
        &bundle,
        100,
    )?;
    let persisted = crate::context_bundle::persistence::load_verified_context_bundle_audit(
        &connection,
        injection_run_id,
    )?
    .expect("persisted ContextAudit");
    let snapshot = context_audit_snapshot(persisted);
    drop(connection);

    let fixture_run = fs::read(root.join("coding/artifacts/smoke-coding-001/run.json"))?;
    let mut run: CodingRunArtifact = serde_json::from_slice(&fixture_run)?;
    run.artifacts = BTreeMap::from([
        (
            "injected_context".to_string(),
            "coding-context.txt".to_string(),
        ),
        (
            "remem_db_snapshot".to_string(),
            "coding-context.sqlite3".to_string(),
        ),
    ]);
    run.injected_context_sha256 =
        Some(format!("{:x}", Sha256::digest(injected_context.as_bytes())));

    let removed = Arc::new(Mutex::new(false));
    let hook_result = Arc::clone(&removed);
    super::set_after_coding_snapshot_consumed_hook(move |path| {
        fs::remove_file(path).expect("remove consumed coding snapshot source");
        *hook_result.lock().expect("lock hook result") = true;
    });
    let mut state = super::super::VerifyState::new(root.clone());

    super::validate_persisted_context_audit_provenance(&run, &snapshot, "coding-run", &mut state);

    assert!(*removed.lock().expect("lock removed flag"));
    assert!(!database_path.exists());
    assert!(state.failures.is_empty(), "{:#?}", state.failures);
    assert!(state.consumed_bytes.contains_key("coding-context.sqlite3"));
    Ok(())
}

fn empty_context_bundle(token_estimate: u32) -> crate::context_bundle::ContextBundle {
    let plan_hash = "a".repeat(64);
    crate::context_bundle::ContextBundle {
        schema_version: crate::context_bundle::CONTEXT_BUNDLE_SCHEMA_VERSION,
        plan_hash: plan_hash.clone(),
        degraded_mode: crate::context_bundle::DegradedMode::Full,
        preferences: Vec::new(),
        failure_lessons: Vec::new(),
        current_truth: Vec::new(),
        workstreams: Vec::new(),
        memory_index: Vec::new(),
        recent_sessions: Vec::new(),
        audit: crate::context_bundle::ContextAudit {
            schema_version: crate::context_bundle::CONTEXT_BUNDLE_SCHEMA_VERSION,
            policy_version: "retrieval_router_v2".to_string(),
            relevance_policy_version: "sessionstart_significant_token_v1".to_string(),
            plan_hash,
            degraded_mode: crate::context_bundle::DegradedMode::Full,
            candidates_considered: 0,
            selected_count: 0,
            dropped_count: 0,
            token_estimate,
            token_budget: 100,
            truncation_reason: None,
            entries: Vec::new(),
            shadow_comparison: Vec::new(),
        },
    }
}

fn context_audit_snapshot(
    persisted: crate::context_bundle::persistence::PersistedContextBundleAudit,
) -> crate::eval::coding_bench::RememContextAuditSnapshot {
    let binding = context_audit_binding_hash(&persisted.injection_run_id, &persisted.audit_hash);
    crate::eval::coding_bench::RememContextAuditSnapshot {
        injection_run_id: persisted.injection_run_id,
        bundle_schema_version: persisted.bundle_schema_version,
        plan_schema_version: persisted.plan_schema_version,
        policy_version: persisted.audit.policy_version,
        relevance_policy_version: persisted.audit.relevance_policy_version,
        plan_hash: persisted.audit.plan_hash,
        audit_hash: persisted.audit_hash,
        injection_binding_hash: binding,
        degraded_mode: persisted.audit.degraded_mode,
        candidates_considered: persisted.audit.candidates_considered,
        selected_count: persisted.audit.selected_count,
        dropped_count: persisted.audit.dropped_count,
        token_budget: persisted.audit.token_budget,
        token_estimate: persisted.audit.token_estimate,
        truncation_reason: persisted.audit.truncation_reason,
        canonical_audit_json: persisted.canonical_audit_json,
    }
}

fn context_audit_binding_hash(injection_run_id: &str, audit_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"remem-coding-bench-context-audit-binding-v1\0");
    hasher.update((injection_run_id.len() as u64).to_be_bytes());
    hasher.update(injection_run_id.as_bytes());
    hasher.update((audit_hash.len() as u64).to_be_bytes());
    hasher.update(audit_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}
