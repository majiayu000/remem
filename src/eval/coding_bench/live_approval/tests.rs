use base64::Engine as _;
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use serde::Serialize;

use super::types::{
    ApprovalTrustRoot, ApprovedCaps, ApprovedLedger, ApprovedPricing, ApprovedRunner,
    ApprovedSigstore, ApprovedSupervisor, ApprovedTuple, CanonicalPlanBinding, DetachedSignature,
    ExpectedLiveBinding, LiveApprovalPayload, RunPhase, SignedSupervisorAttestation,
    SupervisorAttestationPayload, VerificationKey,
};
use super::*;

const NOW: i64 = 1_800_000_000;

#[test]
fn signed_approval_and_supervisor_attestation_validate() {
    let approval_key = key_pair(7);
    let supervisor_key = key_pair(11);
    let expected = expected_binding();
    let approval = approval_payload(&expected);
    let attestation = supervisor_attestation(&approval, &expected, &supervisor_key);
    let trust_root = trust_root(&approval_key, &supervisor_key);

    validate_trust_root(&trust_root).expect("trust root should validate");
    verify_detached_signature(
        &approval,
        &sign(&approval, "approval-v1", &approval_key),
        &trust_root.approval_key,
        "approval",
    )
    .expect("approval signature should validate");
    verify_detached_signature(
        &attestation.payload,
        &attestation.signature,
        &trust_root.supervisor_key,
        "supervisor",
    )
    .expect("supervisor signature should validate");
    validate_approval_payload(&approval, &expected, NOW).expect("approval payload should validate");
    validate_supervisor_attestation(&attestation, &approval, &expected, NOW)
        .expect("supervisor attestation should validate");
}

#[test]
fn tampered_signature_is_rejected() {
    let approval_key = key_pair(7);
    let trust_key = verification_key("approval-v1", &approval_key);
    let payload = approval_payload(&expected_binding());
    let mut signature = sign(&payload, "approval-v1", &approval_key);
    signature.signature_base64 = base64::engine::general_purpose::STANDARD.encode([0_u8; 64]);

    let error = verify_detached_signature(&payload, &signature, &trust_key, "approval")
        .expect_err("tampered signature must fail");
    assert!(error.to_string().contains("signature verification failed"));
}

#[test]
fn expired_caps_and_plan_mismatches_are_rejected() {
    let expected = expected_binding();

    let mut expired = approval_payload(&expected);
    expired.expires_at_epoch = NOW;
    assert!(validate_approval_payload(&expired, &expected, NOW)
        .expect_err("expired approval must fail")
        .to_string()
        .contains("expired"));

    let mut excessive_call_identity = approval_payload(&expected);
    excessive_call_identity.caps.max_agent_calls += 1;
    assert!(
        validate_approval_payload(&excessive_call_identity, &expected, NOW)
            .expect_err("agent-call cap mismatch must fail")
            .to_string()
            .contains("agent-call cap")
    );

    let mut wrong_plan = approval_payload(&expected);
    wrong_plan.plan_sha256 = digest('f');
    assert!(validate_approval_payload(&wrong_plan, &expected, NOW)
        .expect_err("plan digest mismatch must fail")
        .to_string()
        .contains("plan digest mismatch"));
}

#[test]
fn phase_contracts_are_closed_and_local_flags_cannot_leak() {
    let mut options = options();
    options.run_phase = "smoke".to_string();
    options.task_set = "smoke".to_string();
    options.runs_per_condition = 1;
    options.matrix_namespace = "smoke-ci".to_string();
    validate_phase_options(&options, RunPhase::Smoke).expect("valid smoke contract");

    options.run_phase = "official".to_string();
    options.task_set = "full".to_string();
    options.runs_per_condition = 3;
    options.matrix_namespace = "official-v1".to_string();
    validate_phase_options(&options, RunPhase::Official).expect("valid official contract");

    options.run_phase = "local".to_string();
    options.matrix_namespace = "official-v1".to_string();
    assert!(validate_local_planning(&options)
        .expect_err("live identity cannot be used by local dry-run")
        .to_string()
        .contains("live approval flags"));

    assert!(parse_phase("preview").is_err());
}

#[tokio::test]
async fn non_local_execution_gate_precedes_fixture_and_runner_access() {
    let mut options = options();
    options.run_phase = "smoke".to_string();
    options.task_set = "smoke".to_string();
    options.runs_per_condition = 1;
    options.matrix_namespace = "smoke-zero-call".to_string();
    options.fixture_path = "/definitely/missing/gh931-fixture.json".to_string();
    options.codex_bin = "/definitely/missing/gh931-runner".to_string();

    let error = super::super::runner::run_coding_bench(&options)
        .await
        .expect_err("non-local dispatch must fail at the approval gate");
    let message = error.to_string();
    assert!(
        !message.contains("gh931-fixture"),
        "fixture was accessed: {message}"
    );
    assert!(
        !message.contains("gh931-runner"),
        "runner was accessed: {message}"
    );
}

fn key_pair(seed_byte: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed_byte; 32]).expect("test key seed")
}

fn verification_key(key_id: &str, key_pair: &Ed25519KeyPair) -> VerificationKey {
    VerificationKey {
        key_id: key_id.to_string(),
        algorithm: "ed25519".to_string(),
        public_key_base64: base64::engine::general_purpose::STANDARD
            .encode(key_pair.public_key().as_ref()),
    }
}

fn trust_root(approval_key: &Ed25519KeyPair, supervisor_key: &Ed25519KeyPair) -> ApprovalTrustRoot {
    ApprovalTrustRoot {
        schema_version: 1,
        approval_key: verification_key("approval-v1", approval_key),
        supervisor_key: verification_key("supervisor-v1", supervisor_key),
    }
}

fn sign<T: Serialize>(payload: &T, key_id: &str, key_pair: &Ed25519KeyPair) -> DetachedSignature {
    DetachedSignature {
        key_id: key_id.to_string(),
        algorithm: "ed25519".to_string(),
        signature_base64: base64::engine::general_purpose::STANDARD.encode(
            key_pair
                .sign(&canonical_bytes(payload).expect("canonical test payload"))
                .as_ref(),
        ),
    }
}

fn expected_binding() -> ExpectedLiveBinding {
    let runner = ApprovedRunner {
        runner: "codex".to_string(),
        model: "gpt-5.5".to_string(),
        provider: "openai".to_string(),
        reasoning_effort: "medium".to_string(),
    };
    ExpectedLiveBinding {
        phase: RunPhase::Smoke,
        namespace: "smoke-ci".to_string(),
        head_commit: "a".repeat(40),
        plan: CanonicalPlanBinding {
            schema: PLAN_SCHEMA.to_string(),
            repository: REPOSITORY.to_string(),
            approved_commit: "a".repeat(40),
            run_phase: RunPhase::Smoke,
            matrix_namespace: "smoke-ci".to_string(),
            matrix: "primary".to_string(),
            task_set: "smoke".to_string(),
            runs_per_condition: 1,
            tuples: vec![ApprovedTuple {
                condition: "no_memory".to_string(),
                task_id: "task-1".to_string(),
                run_index: 0,
            }],
            fixture_sha256: digest('1'),
            condition_registry_sha256: digest('2'),
            remem_executable_sha256: digest('3'),
            runner_executable_sha256: digest('4'),
            memory_config_sha256: digest('5'),
            curator_manifest_sha256: digest('6'),
            runner,
        },
        plan_sha256: digest('7'),
        supervisor_executable_sha256: digest('8'),
        supervisor_uid: 0,
    }
}

fn approval_payload(expected: &ExpectedLiveBinding) -> LiveApprovalPayload {
    LiveApprovalPayload {
        approval_id: "approval-1".to_string(),
        repository: REPOSITORY.to_string(),
        default_branch: DEFAULT_BRANCH.to_string(),
        approved_commit: expected.head_commit.clone(),
        run_phase: expected.phase,
        matrix_namespace: expected.namespace.clone(),
        plan_sha256: expected.plan_sha256.clone(),
        fixture_sha256: expected.plan.fixture_sha256.clone(),
        condition_registry_sha256: expected.plan.condition_registry_sha256.clone(),
        remem_executable_sha256: expected.plan.remem_executable_sha256.clone(),
        runner_executable_sha256: expected.plan.runner_executable_sha256.clone(),
        memory_config_sha256: expected.plan.memory_config_sha256.clone(),
        curator_manifest_sha256: expected.plan.curator_manifest_sha256.clone(),
        runner: expected.plan.runner.clone(),
        pricing: ApprovedPricing {
            currency: "USD".to_string(),
            input_usd_micros_per_million_tokens: 1,
            output_usd_micros_per_million_tokens: 2,
        },
        caps: ApprovedCaps {
            max_agent_calls: 1,
            max_provider_calls: 1,
            max_input_tokens: 10_000,
            max_output_tokens: 2_000,
            max_cost_usd_micros: 5_000_000,
        },
        supervisor: ApprovedSupervisor {
            identity: "gh931-supervisor".to_string(),
            signing_key_id: "supervisor-v1".to_string(),
            executable_sha256: expected.supervisor_executable_sha256.clone(),
            required_uid: 0,
        },
        ledger: ApprovedLedger {
            writer_app_id: 1,
            writer_app_slug: "gh931-ledger-writer".to_string(),
            signing_key_id: "ledger-v1".to_string(),
            ledger_ref: "refs/heads/gh931-ledger".to_string(),
            update_ruleset_id: 11,
            update_ruleset_sha256: digest('9'),
            no_bypass_ruleset_id: 12,
            no_bypass_ruleset_sha256: digest('a'),
        },
        sigstore: ApprovedSigstore {
            trusted_root_sha256: digest('b'),
            signing_config_sha256: digest('c'),
            rekor_log_id: digest('d'),
        },
        not_before_epoch: NOW - 60,
        expires_at_epoch: NOW + 60,
    }
}

fn supervisor_attestation(
    approval: &LiveApprovalPayload,
    expected: &ExpectedLiveBinding,
    key_pair: &Ed25519KeyPair,
) -> SignedSupervisorAttestation {
    let payload = SupervisorAttestationPayload {
        approval_id: approval.approval_id.clone(),
        plan_sha256: expected.plan_sha256.clone(),
        supervisor_identity: approval.supervisor.identity.clone(),
        supervisor_executable_sha256: expected.supervisor_executable_sha256.clone(),
        supervisor_uid: 0,
        platform: "linux".to_string(),
        nofollow_open: true,
        same_handle_execution: true,
        caller_cannot_access_signing_key: true,
        not_before_epoch: NOW - 30,
        expires_at_epoch: NOW + 30,
    };
    let signature = sign(&payload, "supervisor-v1", key_pair);
    SignedSupervisorAttestation {
        schema_version: 1,
        payload,
        signature,
    }
}

fn digest(character: char) -> String {
    character.to_string().repeat(64)
}

fn options() -> CodingBenchOptions {
    CodingBenchOptions {
        fixture_path: "eval/coding-bench/fixtures/tasks.json".to_string(),
        runs_per_condition: 3,
        json_out: "/tmp/gh931-report.json".to_string(),
        condition: None,
        matrix: "primary".to_string(),
        task: None,
        task_set: "full".to_string(),
        keep_workdirs: false,
        dry_run: false,
        runner: "codex".to_string(),
        codex_bin: "codex".to_string(),
        model: "gpt-5.5".to_string(),
        provider: Some("openai".to_string()),
        reasoning_effort: "medium".to_string(),
        ignore_budget: false,
        curator_root: Some("/tmp/curator".to_string()),
        memory_config: Some("/tmp/remem.toml".to_string()),
        run_phase: "local".to_string(),
        matrix_namespace: "local".to_string(),
        verify_live_approval_only: false,
        live_approval: None,
        approval_trust_root: None,
        supervisor_attestation: None,
        supervisor_bin: None,
    }
}
