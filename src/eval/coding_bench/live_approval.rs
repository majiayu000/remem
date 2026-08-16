mod io;
#[cfg(test)]
mod tests;
mod types;

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use base64::Engine as _;
use ring::signature;
use serde::Serialize;

use super::dry_run::effective_matrix;
use super::fixture::{load_fixture, selected_conditions, selected_tasks};
use super::run_plan::build_run_plan;
use super::types::CodingBenchOptions;
use io::{
    ensure_ancestor, load_git_binding, read_json_nofollow, read_nofollow, read_tracked_head_json,
    resolve_executable, sha256_hex, FileSnapshot,
};
use types::{
    ApprovalTrustRoot, ApprovedCaps, ApprovedLedger, ApprovedPricing, ApprovedRunner,
    ApprovedSigstore, CanonicalPlanBinding, DetachedSignature, ExpectedLiveBinding,
    LiveApprovalPayload, LiveApprovalReport, RunPhase, SignedApprovalEnvelope,
    SignedSupervisorAttestation, VerificationKey,
};

const REPOSITORY: &str = "majiayu000/remem";
const DEFAULT_BRANCH: &str = "main";
const CONDITION_REGISTRY: &str = "eval/coding-bench/conditions.json";
const PLAN_SCHEMA: &str = "gh931_live_plan_v1";

fn parse_phase(value: &str) -> Result<RunPhase> {
    RunPhase::parse(value).with_context(|| {
        format!("unsupported --run-phase {value:?}; expected local, smoke, or official")
    })
}

struct VerificationEnvironment {
    cwd: PathBuf,
    current_exe: PathBuf,
    platform: String,
    now_epoch: i64,
    enforce_root_supervisor: bool,
}

impl VerificationEnvironment {
    fn production() -> Result<Self> {
        Ok(Self {
            cwd: std::env::current_dir().context("resolve live approval working directory")?,
            current_exe: std::env::current_exe()
                .context("resolve current remem executable for live approval")?,
            platform: std::env::consts::OS.to_string(),
            now_epoch: chrono::Utc::now().timestamp(),
            enforce_root_supervisor: true,
        })
    }
}

pub(super) fn verify_live_approval_json(options: &CodingBenchOptions) -> Result<String> {
    let environment = VerificationEnvironment::production()?;
    let report = verify_live_approval(options, &environment)?;
    serde_json::to_string_pretty(&report).map_err(Into::into)
}

pub(super) fn enforce_execution_gate(options: &CodingBenchOptions) -> Result<()> {
    let phase = parse_phase(&options.run_phase)?;
    if phase == RunPhase::Local {
        reject_live_flags_in_local_mode(options)?;
        return Ok(());
    }
    let environment = VerificationEnvironment::production()?;
    let _ = verify_live_approval(options, &environment)?;
    bail!(
        "GH931 live approval passed the repository-local gate, but non-local dispatch remains disabled until the independent governed executor, scorer, and ledger/TUF/Rekor authority are integrated"
    )
}

pub(super) fn validate_local_planning(options: &CodingBenchOptions) -> Result<()> {
    let phase = parse_phase(&options.run_phase)?;
    if phase != RunPhase::Local {
        bail!(
            "non-local dry runs cannot establish a GH931 run identity; use --verify-live-approval-only"
        );
    }
    reject_live_flags_in_local_mode(options)
}

fn verify_live_approval(
    options: &CodingBenchOptions,
    environment: &VerificationEnvironment,
) -> Result<LiveApprovalReport> {
    let phase = parse_phase(&options.run_phase)?;
    if phase == RunPhase::Local {
        bail!("--verify-live-approval-only requires --run-phase smoke or official");
    }
    validate_phase_options(options, phase)?;
    if environment.platform != "linux" {
        bail!(
            "GH931 live approval is unsupported on platform {}; a reviewed same-handle supervisor protocol is required",
            environment.platform
        );
    }

    let git = load_git_binding(&environment.cwd)?;
    ensure!(
        git.branch == DEFAULT_BRANCH,
        "GH931 live approval requires current branch {DEFAULT_BRANCH}, found {}",
        git.branch
    );
    ensure!(
        git.head == git.origin_main,
        "GH931 live approval requires HEAD to equal locally known origin/main"
    );

    let trust_root_path = required_path(
        options.approval_trust_root.as_deref(),
        "--approval-trust-root",
    )?;
    let approval_path = required_path(options.live_approval.as_deref(), "--live-approval")?;
    let attestation_path = required_path(
        options.supervisor_attestation.as_deref(),
        "--supervisor-attestation",
    )?;
    let (trust_root, _) = read_tracked_head_json::<ApprovalTrustRoot>(
        &git,
        trust_root_path,
        "GH931 approval trust root",
    )?;
    validate_trust_root(&trust_root)?;
    let (approval, _) = read_tracked_head_json::<SignedApprovalEnvelope>(
        &git,
        approval_path,
        "GH931 live approval",
    )?;
    ensure!(
        approval.schema_version == 1,
        "unsupported live approval schema"
    );
    verify_detached_signature(
        &approval.payload,
        &approval.signature,
        &trust_root.approval_key,
        "live approval",
    )?;
    ensure_ancestor(&git, &approval.payload.approved_commit)?;

    let expected = build_expected_binding(
        options,
        environment,
        &git.root,
        phase,
        &approval.payload.approved_commit,
    )?;
    let (attestation, attestation_snapshot) = read_json_nofollow::<SignedSupervisorAttestation>(
        attestation_path,
        "GH931 supervisor attestation",
    )?;
    ensure!(
        attestation.schema_version == 1,
        "unsupported supervisor attestation schema"
    );
    verify_detached_signature(
        &attestation.payload,
        &attestation.signature,
        &trust_root.supervisor_key,
        "supervisor attestation",
    )?;
    ensure!(
        approval.payload.supervisor.signing_key_id == trust_root.supervisor_key.key_id,
        "approved supervisor signing key is not the trust-root supervisor key"
    );
    validate_approval_payload(&approval.payload, &expected, environment.now_epoch)?;
    validate_supervisor_attestation(
        &attestation,
        &approval.payload,
        &expected,
        environment.now_epoch,
    )?;

    Ok(LiveApprovalReport {
        schema_version: 1,
        passed: true,
        gate_scope: "local_gate_only".to_string(),
        dispatch_authorized: false,
        provider_or_agent_calls: 0,
        approval_id: approval.payload.approval_id.clone(),
        run_phase: phase,
        matrix_namespace: expected.namespace,
        approved_commit: expected.head_commit,
        plan_sha256: expected.plan_sha256,
        approval_payload_sha256: canonical_sha256(&approval.payload)?,
        supervisor_attestation_sha256: attestation_snapshot.sha256,
        approval_expires_at_epoch: approval.payload.expires_at_epoch,
        supervisor_attestation_expires_at_epoch: attestation.payload.expires_at_epoch,
        checks: vec![
            "default_branch_blobs_bound".to_string(),
            "approval_signature_verified".to_string(),
            "approved_commit_is_ancestor".to_string(),
            "plan_and_artifact_hashes_bound".to_string(),
            "pricing_and_hard_caps_valid".to_string(),
            "supervisor_signature_and_binary_bound".to_string(),
            "security_clock_validity_verified".to_string(),
            "dispatch_disabled_pending_external_authority".to_string(),
        ],
    })
}

fn build_expected_binding(
    options: &CodingBenchOptions,
    environment: &VerificationEnvironment,
    repository_root: &Path,
    phase: RunPhase,
    approved_commit: &str,
) -> Result<ExpectedLiveBinding> {
    validate_digest_or_commit(approved_commit, 40, "approved_commit")?;
    let fixture = load_fixture(&options.fixture_path)?;
    let conditions = selected_conditions(options)?;
    let tasks = selected_tasks(&fixture, options)?;
    ensure!(
        conditions.as_slice() == super::types::BenchCondition::PRIMARY.as_slice(),
        "GH931 live approval requires the exact primary condition set"
    );
    super::preflight::validate_condition_inputs(options, &conditions, &tasks)?;

    let fixture_path = resolve_input_path(&environment.cwd, &options.fixture_path);
    let fixture_snapshot = read_nofollow(&fixture_path, "GH931 fixture")?;
    let condition_registry = read_nofollow(
        &repository_root.join(CONDITION_REGISTRY),
        "condition registry",
    )?;
    let remem_executable = read_nofollow(&environment.current_exe, "remem executable")?;
    let runner_path = resolve_executable(&options.codex_bin, &environment.cwd)?;
    let runner_executable = read_nofollow(&runner_path, "coding runner executable")?;
    let memory_config_path = required_path(options.memory_config.as_deref(), "--memory-config")?;
    let memory_config = read_nofollow(
        &resolve_path(&environment.cwd, memory_config_path),
        "remem memory config",
    )?;
    let curator_root = required_path(options.curator_root.as_deref(), "--curator-root")?;
    let curator_root = resolve_path(&environment.cwd, curator_root);
    let curator_manifest_sha256 =
        super::curator::budgeted_input_manifest_sha256(&curator_root, &tasks)?;
    let supervisor_path = required_path(options.supervisor_bin.as_deref(), "--supervisor-bin")?;
    let supervisor_path = supervisor_path
        .to_str()
        .context("supervisor executable path is not UTF-8")?;
    let supervisor_path = resolve_executable(supervisor_path, &environment.cwd)?;
    let supervisor = read_nofollow(&supervisor_path, "GH931 supervisor executable")?;
    validate_supervisor_file(&supervisor, environment.enforce_root_supervisor)?;

    let canonical_plan = build_run_plan(&conditions, tasks.len(), options.runs_per_condition);
    let tuples = canonical_plan
        .into_iter()
        .map(|entry| types::ApprovedTuple {
            condition: entry.condition.as_str().to_string(),
            task_id: tasks[entry.task_index].id.clone(),
            run_index: entry.run_index,
        })
        .collect();
    let runner = ApprovedRunner {
        runner: options.runner.clone(),
        model: options.model.clone(),
        provider: options
            .provider
            .clone()
            .unwrap_or_else(|| options.runner.clone()),
        reasoning_effort: options.reasoning_effort.clone(),
    };
    let namespace = options.matrix_namespace.trim().to_string();
    let plan = CanonicalPlanBinding {
        schema: PLAN_SCHEMA.to_string(),
        repository: REPOSITORY.to_string(),
        approved_commit: approved_commit.to_ascii_lowercase(),
        run_phase: phase,
        matrix_namespace: namespace.clone(),
        matrix: effective_matrix(options).to_string(),
        task_set: options.task_set.trim().to_string(),
        runs_per_condition: options.runs_per_condition,
        tuples,
        fixture_sha256: fixture_snapshot.sha256,
        condition_registry_sha256: condition_registry.sha256,
        remem_executable_sha256: remem_executable.sha256,
        runner_executable_sha256: runner_executable.sha256,
        memory_config_sha256: memory_config.sha256,
        curator_manifest_sha256,
        runner,
    };
    let plan_sha256 = canonical_sha256(&plan)?;
    Ok(ExpectedLiveBinding {
        phase,
        namespace,
        head_commit: approved_commit.to_ascii_lowercase(),
        plan,
        plan_sha256,
        supervisor_executable_sha256: supervisor.sha256,
        supervisor_uid: supervisor.uid,
    })
}

fn validate_phase_options(options: &CodingBenchOptions, phase: RunPhase) -> Result<()> {
    ensure!(
        !options.ignore_budget,
        "live runs cannot use --ignore-budget"
    );
    ensure!(
        options.condition.is_none() && options.task.is_none(),
        "live approval does not permit caller-selected condition or task subsets"
    );
    ensure!(
        options.matrix.trim() == "primary",
        "live approval requires --matrix primary"
    );
    for (value, label) in [
        (options.runner.as_str(), "runner"),
        (options.model.as_str(), "model"),
        (options.reasoning_effort.as_str(), "reasoning effort"),
    ] {
        validate_identifier(value, label)?;
    }
    if let Some(provider) = options.provider.as_deref() {
        validate_identifier(provider, "provider")?;
    }
    validate_identifier(&options.matrix_namespace, "matrix namespace")?;
    match phase {
        RunPhase::Local => bail!("local phase has no live approval identity"),
        RunPhase::Smoke => {
            ensure!(
                options.task_set.trim() == "smoke",
                "smoke phase requires --task-set smoke"
            );
            ensure!(
                options.runs_per_condition == 1,
                "smoke phase requires one run per tuple"
            );
            ensure!(
                options.matrix_namespace.starts_with("smoke-")
                    && options.matrix_namespace != "official-v1",
                "smoke phase requires a non-official smoke-* namespace"
            );
        }
        RunPhase::Official => {
            ensure!(
                options.task_set.trim() == "full",
                "official phase requires --task-set full"
            );
            ensure!(
                options.runs_per_condition == 3,
                "official phase requires three runs per tuple"
            );
            ensure!(
                options.matrix_namespace == "official-v1",
                "official phase requires matrix namespace official-v1"
            );
        }
    }
    Ok(())
}

fn reject_live_flags_in_local_mode(options: &CodingBenchOptions) -> Result<()> {
    if options.verify_live_approval_only
        || options.live_approval.is_some()
        || options.approval_trust_root.is_some()
        || options.supervisor_attestation.is_some()
        || options.supervisor_bin.is_some()
        || options.matrix_namespace != "local"
    {
        bail!("live approval flags require --run-phase smoke or official");
    }
    Ok(())
}

fn validate_trust_root(root: &ApprovalTrustRoot) -> Result<()> {
    ensure!(
        root.schema_version == 1,
        "unsupported approval trust-root schema"
    );
    validate_verification_key(&root.approval_key, "approval key")?;
    validate_verification_key(&root.supervisor_key, "supervisor key")?;
    ensure!(
        root.approval_key.key_id != root.supervisor_key.key_id,
        "approval and supervisor keys must be distinct"
    );
    let approval_public_key =
        decode_base64(&root.approval_key.public_key_base64, "approval public key")?;
    let supervisor_public_key = decode_base64(
        &root.supervisor_key.public_key_base64,
        "supervisor public key",
    )?;
    ensure!(
        approval_public_key != supervisor_public_key,
        "approval and supervisor public keys must be distinct"
    );
    Ok(())
}

fn validate_verification_key(key: &VerificationKey, label: &str) -> Result<()> {
    validate_identifier(&key.key_id, &format!("{label} id"))?;
    ensure!(key.algorithm == "ed25519", "{label} must use ed25519");
    let bytes = decode_base64(&key.public_key_base64, &format!("{label} public key"))?;
    ensure!(bytes.len() == 32, "{label} public key must be 32 bytes");
    Ok(())
}

fn verify_detached_signature<T: Serialize>(
    payload: &T,
    detached: &DetachedSignature,
    key: &VerificationKey,
    label: &str,
) -> Result<()> {
    ensure!(detached.key_id == key.key_id, "{label} key id mismatch");
    ensure!(detached.algorithm == "ed25519", "{label} must use ed25519");
    let public_key = decode_base64(&key.public_key_base64, "Ed25519 public key")?;
    let signature_bytes = decode_base64(&detached.signature_base64, label)?;
    let payload_bytes = canonical_bytes(payload)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
        .verify(&payload_bytes, &signature_bytes)
        .map_err(|_| anyhow::anyhow!("{label} signature verification failed"))
}

fn validate_approval_payload(
    payload: &LiveApprovalPayload,
    expected: &ExpectedLiveBinding,
    now_epoch: i64,
) -> Result<()> {
    validate_identifier(&payload.approval_id, "approval_id")?;
    ensure!(
        payload.repository == REPOSITORY,
        "approval repository mismatch"
    );
    ensure!(
        payload.default_branch == DEFAULT_BRANCH,
        "approval default branch mismatch"
    );
    ensure!(
        payload.run_phase == expected.phase,
        "approval run phase mismatch"
    );
    ensure!(
        payload.matrix_namespace == expected.namespace,
        "approval namespace mismatch"
    );
    ensure!(
        payload.approved_commit == expected.head_commit,
        "approval commit mismatch"
    );
    ensure!(
        payload.plan_sha256 == expected.plan_sha256,
        "approval plan digest mismatch"
    );
    ensure!(
        payload.fixture_sha256 == expected.plan.fixture_sha256,
        "fixture digest mismatch"
    );
    ensure!(
        payload.condition_registry_sha256 == expected.plan.condition_registry_sha256,
        "condition registry digest mismatch"
    );
    ensure!(
        payload.remem_executable_sha256 == expected.plan.remem_executable_sha256,
        "remem executable digest mismatch"
    );
    ensure!(
        payload.runner_executable_sha256 == expected.plan.runner_executable_sha256,
        "runner executable digest mismatch"
    );
    ensure!(
        payload.memory_config_sha256 == expected.plan.memory_config_sha256,
        "memory config digest mismatch"
    );
    ensure!(
        payload.curator_manifest_sha256 == expected.plan.curator_manifest_sha256,
        "curator manifest digest mismatch"
    );
    ensure!(
        payload.runner == expected.plan.runner,
        "runner profile binding mismatch"
    );
    validate_validity_window(
        payload.not_before_epoch,
        payload.expires_at_epoch,
        now_epoch,
        "approval",
    )?;
    validate_pricing(&payload.pricing)?;
    validate_caps(&payload.caps, expected.plan.tuples.len())?;
    validate_supervisor_approval(payload, expected)?;
    validate_ledger(&payload.ledger)?;
    validate_sigstore(&payload.sigstore)?;
    Ok(())
}

fn validate_supervisor_approval(
    payload: &LiveApprovalPayload,
    expected: &ExpectedLiveBinding,
) -> Result<()> {
    validate_identifier(&payload.supervisor.identity, "supervisor identity")?;
    validate_identifier(
        &payload.supervisor.signing_key_id,
        "supervisor signing key id",
    )?;
    validate_digest(
        &payload.supervisor.executable_sha256,
        "supervisor executable digest",
    )?;
    ensure!(
        payload.supervisor.executable_sha256 == expected.supervisor_executable_sha256,
        "supervisor executable digest mismatch"
    );
    ensure!(
        payload.supervisor.required_uid == 0,
        "supervisor approval must require uid 0"
    );
    ensure!(
        expected.supervisor_uid == 0,
        "supervisor executable is not root-owned"
    );
    Ok(())
}

fn validate_supervisor_attestation(
    attestation: &SignedSupervisorAttestation,
    approval: &LiveApprovalPayload,
    expected: &ExpectedLiveBinding,
    now_epoch: i64,
) -> Result<()> {
    let payload = &attestation.payload;
    ensure!(
        payload.approval_id == approval.approval_id,
        "supervisor approval id mismatch"
    );
    ensure!(
        payload.plan_sha256 == expected.plan_sha256,
        "supervisor plan digest mismatch"
    );
    ensure!(
        payload.supervisor_identity == approval.supervisor.identity,
        "supervisor identity mismatch"
    );
    ensure!(
        payload.supervisor_executable_sha256 == expected.supervisor_executable_sha256,
        "supervisor attested executable digest mismatch"
    );
    ensure!(
        payload.supervisor_uid == 0,
        "supervisor attestation must bind uid 0"
    );
    ensure!(
        payload.platform == "linux",
        "supervisor attestation must bind Linux"
    );
    ensure!(
        payload.nofollow_open,
        "supervisor attestation omitted no-follow open"
    );
    ensure!(
        payload.same_handle_execution,
        "supervisor attestation omitted same-handle execution"
    );
    ensure!(
        payload.caller_cannot_access_signing_key,
        "supervisor signing key is not caller-isolated"
    );
    validate_validity_window(
        payload.not_before_epoch,
        payload.expires_at_epoch,
        now_epoch,
        "supervisor attestation",
    )?;
    ensure!(
        payload.not_before_epoch >= approval.not_before_epoch
            && payload.expires_at_epoch <= approval.expires_at_epoch,
        "supervisor attestation validity must be contained by approval validity"
    );
    Ok(())
}

fn validate_pricing(pricing: &ApprovedPricing) -> Result<()> {
    ensure!(
        pricing.currency == "USD",
        "approval pricing currency must be USD"
    );
    ensure!(
        pricing.input_usd_micros_per_million_tokens > 0
            && pricing.output_usd_micros_per_million_tokens > 0,
        "approval token pricing must be positive"
    );
    Ok(())
}

fn validate_caps(caps: &ApprovedCaps, tuple_count: usize) -> Result<()> {
    let tuple_count = u64::try_from(tuple_count).context("convert approved tuple count")?;
    ensure!(
        caps.max_agent_calls == tuple_count,
        "agent-call cap must equal tuple count"
    );
    ensure!(
        caps.max_provider_calls >= caps.max_agent_calls,
        "provider-call cap is too small"
    );
    ensure!(
        caps.max_input_tokens > 0 && caps.max_output_tokens > 0 && caps.max_cost_usd_micros > 0,
        "token and cost caps must be positive"
    );
    Ok(())
}

fn validate_ledger(ledger: &ApprovedLedger) -> Result<()> {
    ensure!(
        ledger.writer_app_id > 0,
        "ledger writer App id must be positive"
    );
    ensure!(
        ledger.update_ruleset_id > 0 && ledger.no_bypass_ruleset_id > 0,
        "ruleset ids must be positive"
    );
    ensure!(
        ledger.update_ruleset_id != ledger.no_bypass_ruleset_id,
        "ledger rulesets must be distinct"
    );
    for (value, label) in [
        (&ledger.writer_app_slug, "ledger writer App slug"),
        (&ledger.signing_key_id, "ledger signing key id"),
        (&ledger.ledger_ref, "ledger ref"),
    ] {
        validate_identifier(value, label)?;
    }
    validate_digest(&ledger.update_ruleset_sha256, "update ruleset digest")?;
    validate_digest(&ledger.no_bypass_ruleset_sha256, "no-bypass ruleset digest")?;
    Ok(())
}

fn validate_sigstore(sigstore: &ApprovedSigstore) -> Result<()> {
    validate_digest(&sigstore.trusted_root_sha256, "Sigstore TrustedRoot digest")?;
    validate_digest(
        &sigstore.signing_config_sha256,
        "Sigstore SigningConfig digest",
    )?;
    validate_digest_or_commit(&sigstore.rekor_log_id, 64, "Rekor log id")
}

fn validate_supervisor_file(snapshot: &FileSnapshot, enforce_root: bool) -> Result<()> {
    if enforce_root {
        ensure!(
            snapshot.uid == 0,
            "supervisor executable must be root-owned"
        );
        ensure!(
            snapshot.mode & 0o022 == 0,
            "supervisor executable must not be group- or world-writable"
        );
        ensure!(
            snapshot.mode & 0o111 != 0,
            "supervisor executable has no executable bit"
        );
    }
    Ok(())
}

fn validate_validity_window(not_before: i64, expires_at: i64, now: i64, label: &str) -> Result<()> {
    ensure!(
        not_before > 0 && expires_at > not_before,
        "{label} validity window is invalid"
    );
    ensure!(now >= not_before, "{label} is not yet valid");
    ensure!(now < expires_at, "{label} has expired");
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    ensure!(
        value == value.trim(),
        "{label} must not have surrounding whitespace"
    );
    ensure!(
        !value.is_empty() && value.len() <= 256,
        "{label} must be 1-256 bytes"
    );
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':')),
        "{label} contains unsupported characters"
    );
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<()> {
    validate_digest_or_commit(value, 64, label)
}

fn validate_digest_or_commit(value: &str, length: usize, label: &str) -> Result<()> {
    ensure!(
        value.len() == length
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "{label} must be {length} lowercase hex characters"
    );
    Ok(())
}

fn canonical_sha256<T: Serialize>(value: &T) -> Result<String> {
    Ok(sha256_hex(&canonical_bytes(value)?))
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    crate::api::mutation::canonical_json_bytes(&serde_json::to_value(value)?)
}

fn decode_base64(value: &str, label: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("decode {label} base64"))
}

fn required_path<'a>(value: Option<&'a str>, flag: &str) -> Result<&'a Path> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Path::new)
        .with_context(|| format!("GH931 live approval requires {flag}"))
}

fn resolve_input_path(cwd: &Path, value: &str) -> PathBuf {
    resolve_path(cwd, Path::new(value))
}

fn resolve_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
