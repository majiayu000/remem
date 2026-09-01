use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use super::{copy_public_fixture, failure_text, mutate_json};
use crate::eval::bench_artifact::{
    verify_benchmark_artifacts, AuthorityStatus, BenchVerifyOptions,
};

#[test]
fn verifier_rejects_placeholder_security_snapshot() -> Result<()> {
    let root = copy_public_fixture("placeholder-security-snapshot")?;
    let snapshot = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/remem.db.snapshot.sqlite3",
    );
    fs::write(&snapshot, b"fixture placeholder\n")?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

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

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("snapshot semantic contract"));
    Ok(())
}

#[test]
fn tampered_security_report_aggregate_fails_closed_against_recomputed_policy() -> Result<()> {
    let root = copy_public_fixture("security-report-aggregate-mismatch")?;
    let report_path = root.join("memory/reports/adversarial-policy-v2.json");
    mutate_json(&report_path, |json| {
        json["aggregate_metrics"]["policy"] = serde_json::json!({
            "non_retention_cases": 999,
            "non_retention_leak_rate": 1.0,
            "false_block_rate": 1.0,
            "suppression_obeyed_rate": 0.0,
            "sensitive_restricted_default_exclusion_rate": 0.0,
            "policy_abstention_accuracy": 0.0,
            "policy_failure_rate": 1.0
        });
    })?;
    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("recomputed security aggregate mismatch"));
    let verdict = serde_json::to_value(&report)?;
    assert_eq!(verdict["authority_verdict"]["security"]["status"], "FAIL");
    Ok(())
}

#[test]
fn tampered_run_policy_declarations_cannot_authorize_security_pass() -> Result<()> {
    let root = copy_public_fixture("security-run-policy-mismatch")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    mutate_json(&run_path, |json| {
        json["metrics"]["policy"] = serde_json::json!({
            "active_claim_count": 999,
            "candidate_count": 999,
            "summary_input_count": 999,
            "policy_failure_count": 0
        });
    })?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("run metric /policy/active_claim_count differs"));
    let verdict = serde_json::to_value(&report)?;
    assert_eq!(verdict["authority_verdict"]["security"]["status"], "FAIL");
    Ok(())
}

#[test]
fn mixed_security_reader_model_identity_fails_closed() -> Result<()> {
    let root = copy_public_fixture("security-report-mixed-reader-model")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    mutate_json(&run_path, |json| {
        json["reader_model"]["model"] = Value::String("different-reader".to_string());
    })?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;
    let authority = verified
        .authority_verdict
        .security
        .reports
        .iter()
        .find(|report| report.report_path == "memory/reports/adversarial-policy-v2.json")
        .context("default adversarial-policy v2 authority")?;

    assert_eq!(authority.status, AuthorityStatus::Fail);
    assert!(authority.diagnostics.iter().any(|message| {
        message.contains("security report runs must share one model execution identity")
    }));
    Ok(())
}

#[test]
fn security_run_prompt_hash_must_match_registered_task_prompt() -> Result<()> {
    let root = copy_public_fixture("security-run-prompt-hash")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    mutate_json(&run_path, |json| {
        json["reader_model"]["prompt_hash"] = Value::String("sha256:invalid".to_string());
    })?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(failure_text(&verified).contains("reader prompt hash differs from typed suite task"));
    Ok(())
}

#[test]
fn security_authority_rejects_semantically_identical_unregistered_suite_bytes() -> Result<()> {
    let root = copy_public_fixture("unregistered-security-suite-bytes")?;
    let suite_path = root.join("memory/suites/adversarial-policy/suite.json");
    let mut suite_bytes = fs::read(&suite_path)?;
    suite_bytes.extend_from_slice(b" \n");
    fs::write(&suite_path, &suite_bytes)?;
    let suite_identity = format!("sha256-raw-suite-v1:{:x}", Sha256::digest(&suite_bytes));

    for report_relative in [
        "memory/reports/adversarial-policy-v2.json",
        "memory/reports/adversarial-policy-v2-linux-x86_64.json",
    ] {
        let report_path = root.join(report_relative);
        let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
        let run_paths = report["run_artifacts"]
            .as_array()
            .context("security report run artifacts")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .context("security run path must be a string")
                    .map(str::to_string)
            })
            .collect::<Result<Vec<_>>>()?;
        mutate_json(&report_path, |json| {
            json["aggregate_metrics"]["suite_content_identity"] =
                Value::String(suite_identity.clone());
        })?;
        for run_path in run_paths {
            mutate_json(&root.join(run_path), |json| {
                json["suite_content_identity"] = Value::String(suite_identity.clone());
            })?;
        }
    }

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(failure_text(&verified).contains("registered adversarial security suite identity"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn verifier_rejects_artifact_symlink_that_escapes_public_root() -> Result<()> {
    use std::os::unix::fs::symlink;

    let root = copy_public_fixture("security-artifact-symlink-escape")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let answer_relative = run["artifacts"]["answer"]
        .as_str()
        .context("security answer path")?;
    let answer_path = root.join(answer_relative);
    let outside_path = root.with_extension("outside-answer.json");
    fs::copy(&answer_path, &outside_path)?;
    fs::remove_file(&answer_path)?;
    symlink(&outside_path, &answer_path)?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(failure_text(&verified).contains("artifact target must stay inside benchmark root"));
    Ok(())
}

#[test]
fn oversized_security_snapshot_is_rejected_before_consumption() -> Result<()> {
    let root = copy_public_fixture("oversized-security-snapshot")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let snapshot_relative = run["artifacts"]["remem_db_snapshot"]
        .as_str()
        .context("security snapshot path")?;
    fs::File::create(root.join(snapshot_relative))?.set_len(64 * 1024 * 1024 + 1)?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(failure_text(&verified).contains("security SQLite snapshot exceeds 67108864 bytes"));
    Ok(())
}

#[test]
fn security_snapshot_rejects_payload_hidden_in_unallocated_page_bytes() -> Result<()> {
    let root = copy_public_fixture("security-snapshot-hidden-page-payload")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let snapshot_relative = run["artifacts"]["remem_db_snapshot"]
        .as_str()
        .context("security snapshot path")?;
    let snapshot_path = root.join(snapshot_relative);
    let mut bytes = fs::read(&snapshot_path)?;
    inject_unallocated_page_payload(&mut bytes, b"hidden private reviewer payload")?;
    fs::write(&snapshot_path, &bytes)?;
    let snapshot_sha = format!("{:x}", Sha256::digest(&bytes));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["remem_db_snapshot"] = Value::String(snapshot_sha);
    })?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(verified.failures.iter().any(|failure| {
        failure.path == snapshot_relative
            && failure.message.contains("canonical SQLite snapshot image")
    }));
    Ok(())
}

fn inject_unallocated_page_payload(bytes: &mut [u8], payload: &[u8]) -> Result<()> {
    let raw_page_size = u16::from_be_bytes([bytes[16], bytes[17]]);
    let page_size = if raw_page_size == 1 {
        65_536
    } else {
        usize::from(raw_page_size)
    };
    for page_start in (0..bytes.len()).step_by(page_size) {
        let header = page_start + usize::from(page_start == 0) * 100;
        let header_size = match bytes.get(header).copied() {
            Some(2 | 5) => 12,
            Some(10 | 13) => 8,
            _ => continue,
        };
        let cell_count = usize::from(u16::from_be_bytes([bytes[header + 3], bytes[header + 4]]));
        let raw_cell_start =
            usize::from(u16::from_be_bytes([bytes[header + 5], bytes[header + 6]]));
        let cell_start = if raw_cell_start == 0 {
            page_size
        } else {
            raw_cell_start
        };
        let gap_start = header + header_size + 2 * cell_count;
        let gap_end = page_start + cell_start;
        if gap_end.saturating_sub(gap_start) >= payload.len() {
            bytes[gap_start..gap_start + payload.len()].copy_from_slice(payload);
            return Ok(());
        }
    }
    anyhow::bail!("fixture has no unallocated b-tree page gap large enough for the payload")
}

#[test]
fn security_report_requires_exact_suite_task_coverage_under_remem_default() -> Result<()> {
    for mutation in ["omitted", "duplicate", "extra", "wrong-condition"] {
        let root = copy_public_fixture(&format!("security-report-coverage-{mutation}"))?;
        let report_path = root.join("memory/reports/adversarial-policy-v2.json");
        let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
        let run_paths = report["run_artifacts"]
            .as_array()
            .context("security report run_artifacts")?;
        let first_run = run_paths[0]
            .as_str()
            .context("first security run path")?
            .to_string();
        let last_run = run_paths
            .last()
            .and_then(Value::as_str)
            .context("last security run path")?
            .to_string();
        match mutation {
            "omitted" => mutate_json(&report_path, |json| {
                json["run_artifacts"].as_array_mut().unwrap().pop();
            })?,
            "duplicate" => mutate_json(&report_path, |json| {
                *json["run_artifacts"]
                    .as_array_mut()
                    .unwrap()
                    .last_mut()
                    .unwrap() = Value::String(first_run.clone());
            })?,
            "extra" => mutate_json(&root.join(last_run), |json| {
                json["task_id"] = Value::String("unregistered-security-task".to_string());
            })?,
            "wrong-condition" => mutate_json(&root.join(last_run), |json| {
                json["condition"] = Value::String("no_memory".to_string());
            })?,
            _ => unreachable!(),
        }

        let verified =
            verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;
        let failures = failure_text(&verified);

        assert!(!verified.passed, "{mutation} report unexpectedly passed");
        assert!(
            failures.contains("security report must cover the exact typed suite task set once")
                || failures.contains("security report runs must all use remem_default"),
            "{mutation} report lacked an explicit coverage diagnostic: {failures}"
        );
    }
    Ok(())
}

#[test]
fn security_report_requires_exact_declared_remem_default_condition() -> Result<()> {
    let root = copy_public_fixture("security-report-declared-condition")?;
    for path in [
        "memory/manifests/adversarial-policy-v2.json",
        "memory/reports/adversarial-policy-v2.json",
    ] {
        mutate_json(&root.join(path), |json| {
            json["conditions"] = serde_json::json!(["remem_default", "no_memory"]);
        })?;
    }

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;
    let authority = verified
        .authority_verdict
        .security
        .reports
        .iter()
        .find(|report| report.report_path == "memory/reports/adversarial-policy-v2.json")
        .context("adversarial-policy v2 authority")?;

    assert!(!verified.passed);
    assert_eq!(authority.status, AuthorityStatus::Fail);
    assert!(authority.diagnostics.iter().any(
        |message| message.contains("security report conditions must be exactly remem_default")
    ));
    Ok(())
}

#[test]
fn policy_abstention_rejects_arbitrary_protected_answer_text() -> Result<()> {
    let root = copy_public_fixture("security-arbitrary-abstention-answer")?;
    let protected_text = "A third party has a private appointment tomorrow afternoon.";
    mutate_security_answer_text(
        &root,
        "third-party-personal-001",
        protected_text,
        Some(protected_text),
    )?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(failure_text(&verified)
        .contains("policy-required abstention must use the canonical safe answer"));
    assert!(verified.authority_verdict.security.policy_failure_count > 0);
    Ok(())
}

#[test]
fn referenced_answer_text_must_match_typed_run_answer_text() -> Result<()> {
    let root = copy_public_fixture("security-answer-text-binding")?;
    mutate_security_answer_text(
        &root,
        "third-party-personal-001",
        "Different harmless referenced answer.",
        None,
    )?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(
        failure_text(&verified).contains("referenced answer text must match typed run answer text")
    );
    Ok(())
}

#[test]
fn leaked_protected_answer_text_fails_recomputed_security_policy() -> Result<()> {
    let root = copy_public_fixture("security-answer-text-leak")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let answer_relative = run["artifacts"]["answer"]
        .as_str()
        .context("security answer artifact path")?;
    let answer_path = root.join(answer_relative);
    mutate_json(&answer_path, |json| {
        json["text"] = Value::String(
            "The leaked key is sk-benchmark-secret-0001 despite abstaining.".to_string(),
        );
    })?;
    let answer_sha256 = format!("{:x}", Sha256::digest(fs::read(&answer_path)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["answer"] = Value::String(answer_sha256);
    })?;

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("protected content leaked in referenced answer text"));
    assert!(report.authority_verdict.security.policy_failure_count > 0);
    Ok(())
}

fn mutate_security_answer_text(
    root: &Path,
    task_id: &str,
    referenced_text: &str,
    typed_text: Option<&str>,
) -> Result<()> {
    let run_path = root.join(format!(
        "memory/artifacts/adversarial-policy-v2/remem_default-{task_id}/run.json"
    ));
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let answer_relative = run["artifacts"]["answer"]
        .as_str()
        .context("security answer artifact path")?;
    let answer_path = root.join(answer_relative);
    mutate_json(&answer_path, |json| {
        json["text"] = Value::String(referenced_text.to_string());
    })?;
    let answer_sha256 = format!("{:x}", Sha256::digest(fs::read(&answer_path)?));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["answer"] = Value::String(answer_sha256);
        if let Some(text) = typed_text {
            json["answer"]["text"] = Value::String(text.to_string());
        }
    })?;
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

    let report =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!report.passed);
    assert!(failure_text(&report).contains("closed-world snapshot inventory"));
    Ok(())
}

#[test]
fn verifier_rejects_snapshot_with_trailing_bytes() -> Result<()> {
    let root = copy_public_fixture("security-snapshot-trailing-bytes")?;
    let run_path = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-secrets-api-key-001/run.json",
    );
    let run: Value = serde_json::from_slice(&fs::read(&run_path)?)?;
    let snapshot_relative = run["artifacts"]["remem_db_snapshot"]
        .as_str()
        .context("security run must name its snapshot")?;
    let snapshot = root.join(snapshot_relative);
    let mut bytes = fs::read(&snapshot)?;
    bytes.extend_from_slice(b"private trailing payload");
    fs::write(&snapshot, &bytes)?;
    let mutated_sha256 = format!("{:x}", Sha256::digest(&bytes));
    mutate_json(&run_path, |json| {
        json["artifact_sha256"]["remem_db_snapshot"] = Value::String(mutated_sha256);
    })?;

    let verified =
        verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))?;

    assert!(!verified.passed);
    assert!(failure_text(&verified).contains("SQLite snapshot length differs from header"));
    Ok(())
}

#[test]
fn baseline_consumes_the_exact_typed_bytes_verified_before_replacement() -> Result<()> {
    let root = copy_public_fixture("verified-bytes-snapshot")?;
    let verified = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;
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
