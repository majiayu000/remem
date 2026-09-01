#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    read_json_artifact, rel_display, require_artifact_key, require_non_blank, scan_private_json,
    validate_artifact_map, validate_environment, VerifyState,
};
use crate::eval::bench_artifact::types::{
    BenchmarkLayer, CodingMemoryContract, CodingRunArtifact, CuratorLogArtifact,
    OfficialCodingMaintenanceEvidence, OfficialCodingTestEvidence, PublicBenchmarkReport,
};
use crate::eval::coding_bench::{
    verify_context_audit_snapshot, verify_snapshot_against_persisted_injection,
    RememContextAuditStatus,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    static AFTER_CODING_SNAPSHOT_CONSUMED: RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(in crate::eval::bench_artifact) fn set_after_coding_snapshot_consumed_hook(
    hook: impl FnOnce(&Path) + 'static,
) {
    AFTER_CODING_SNAPSHOT_CONSUMED.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "coding snapshot consumption hook already set"
        );
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_after_coding_snapshot_consumed_hook(path: &Path) {
    AFTER_CODING_SNAPSHOT_CONSUMED.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(path);
        }
    });
}

#[cfg(not(test))]
fn run_after_coding_snapshot_consumed_hook(_path: &Path) {}

const CODING_ARTIFACT_KEYS: [&str; 3] = ["patch", "tool_log", "test_log"];

const PUBLIC_CODING_CONDITIONS: [&str; 11] = [
    "no_memory",
    "curated_file_budgeted",
    "remem_e2e",
    "remem_seeded_sessionstart",
    "curated_file_expert",
    "oracle_evidence",
    "remem_oracle_retrieval",
    "full_history",
    "remem_no_enrichment",
    "remem_fts_only",
    "remem_preloaded",
];

const CODING_FAILURE_REASONS: [&str; 11] = [
    "test_failure",
    "timeout",
    "compile_failure",
    "wrong_file_modified",
    "ignored_memory",
    "missing_memory",
    "stale_memory_followed",
    "irrelevant_memory_distracted",
    "over_context_budget",
    "agent_hallucinated_memory",
    "oracle_inconclusive",
];

pub(super) fn validate_coding_run_artifact(
    run_path: &Path,
    report: &PublicBenchmarkReport,
    _manifest_path: &Path,
    state: &mut VerifyState,
) -> Option<String> {
    state.run_artifacts_checked += 1;
    let raw_artifact = read_json_artifact::<Value>(run_path, state, "coding run artifact")?;
    let raw_run = raw_artifact.value;
    let run = match serde_json::from_value::<CodingRunArtifact>(raw_run.clone()) {
        Ok(run) => run,
        Err(error) => {
            state.fail(
                rel_display(&state.root, run_path),
                format!("validate coding run artifact schema: {error}"),
            );
            return None;
        }
    };
    let run_logical_path = raw_artifact.path.clone();
    state.verified_artifacts.coding_runs.push(
        crate::eval::bench_artifact::types::VerifiedArtifact {
            path: raw_artifact.path,
            sha256: raw_artifact.sha256,
            value: run.clone(),
        },
    );
    let label = rel_display(&state.root, run_path);
    if run.schema_version != 1 {
        state.fail(label.clone(), "coding run schema_version must be 1");
    }
    require_non_blank(&run.benchmark_id, &label, "benchmark_id", state);
    require_non_blank(&run.benchmark_version, &label, "benchmark_version", state);
    require_non_blank(&run.run_phase, &label, "run_phase", state);
    require_non_blank(&run.matrix_namespace, &label, "matrix_namespace", state);
    if run.benchmark_id != report.benchmark_id
        || run.benchmark_version != report.benchmark_version
        || Some(run.run_phase.as_str()) != report.run_phase.as_deref()
        || Some(run.matrix_namespace.as_str()) != report.matrix_namespace.as_deref()
    {
        state.fail(
            label.clone(),
            "coding run benchmark identity must match its report",
        );
    }
    if run.layer != BenchmarkLayer::CodingAgentOutcome || run.layer != report.layer {
        state.fail(
            label.clone(),
            "coding run layer must be coding_agent_outcome",
        );
    }
    require_non_blank(&run.benchmark_version, &label, "benchmark_version", state);
    require_non_blank(&run.condition, &label, "condition", state);
    if !is_public_coding_condition(&run.condition) {
        state.fail(label.clone(), "coding run has unknown condition identity");
    }
    if !report.conditions.contains(&run.condition) {
        state.fail(
            label.clone(),
            "coding run condition must be declared by its report",
        );
    }
    require_non_blank(&run.task_id, &label, "task_id", state);
    let run_key = format!(
        "{}\0{}\0{}\0{}\0{}",
        report.benchmark_id, report.benchmark_version, run.condition, run.task_id, run.run_index
    );
    if !state.coding_run_keys.insert(run_key) {
        state.fail(
            label.clone(),
            "coding task/condition/run_index key must be unique within a benchmark version",
        );
    }
    validate_attempt_state(&run, &raw_run, &label, state);
    validate_environment(&run.environment, &label, state);
    if run.model.is_null() {
        state.fail(label.clone(), "coding run model must be present");
    }
    validate_outcome(&run, &label, state);
    validate_coding_metrics(&run, &label, state);
    validate_artifact_map(&run.artifacts, CODING_ARTIFACT_KEYS, &label, state);
    if is_gh931_official_run(&run) {
        validate_official_test_evidence(&run, &run_logical_path, &label, state);
        if run.condition == "remem_e2e" {
            require_artifact_key(&run.artifacts, "maintenance_evidence", &label, state);
            validate_treatment_maintenance(&run, &run_logical_path, &label, state);
        }
    }
    if is_gh931_official_curated_run(&run) {
        validate_curator_evidence(&run, &run_logical_path, &label, state);
    }
    if requires_remem_evidence(&run.condition) {
        require_artifact_key(&run.artifacts, "injected_context", &label, state);
        require_artifact_key(&run.artifacts, "remem_db_snapshot", &label, state);
        if let Some(contract) = &run.memory_contract {
            validate_coding_memory_contract(contract, run.failure_reason.as_deref(), &label, state);
        } else {
            state.fail(
                label.clone(),
                "remem-backed coding run must include memory_contract",
            );
        }
    } else if run.memory_contract.is_some() {
        state.fail(
            label.clone(),
            "non-remem coding condition must not include memory_contract",
        );
    }
    validate_coding_context_audit(&run, &raw_run, &label, state);
    scan_private_json(
        &serde_json::to_value(&run).unwrap_or(Value::Null),
        run_path,
        "$",
        state,
    );
    Some(run.condition)
}

fn is_gh931_official_run(run: &CodingRunArtifact) -> bool {
    run.benchmark_id == "issue385-v1"
        && run.benchmark_version == "official-v1"
        && run.run_phase == "official"
        && run.matrix_namespace == "issue385-v1/official-v1"
}

fn is_gh931_official_curated_run(run: &CodingRunArtifact) -> bool {
    is_gh931_official_run(run) && run.condition == "curated_file_budgeted"
}

fn validate_official_test_evidence(
    run: &CodingRunArtifact,
    run_path: &str,
    label: &str,
    state: &mut VerifyState,
) {
    let Some(path) = artifact_file_path(run, "test_log", label, state) else {
        return;
    };
    let Some(evidence) = read_json_artifact::<OfficialCodingTestEvidence>(
        &path,
        state,
        "official coding test evidence",
    ) else {
        return;
    };
    let value = &evidence.value;
    if value.schema_version != 1
        || value.task_id != run.task_id
        || value.condition != run.condition
        || value.run_index != run.run_index
        || Some(value.attempt_id.as_str()) != run.attempt_id.as_deref()
        || value.attempt_id.trim().is_empty()
    {
        state.fail(
            label.to_string(),
            "official coding test evidence identity is invalid",
        );
        return;
    }
    if let Some(message) = value.command_validation_error() {
        state.fail(label.to_string(), message);
        return;
    }
    if !value.matches_registered_scorer_commands(&run.task_id) {
        state.fail(
            label.to_string(),
            "official coding test evidence differs from the registered scorer command set",
        );
        return;
    }
    let failure_reason = value.failure_reason();
    if run.resolved != value.resolved() || run.failure_reason.as_deref() != failure_reason {
        state.fail(
            label.to_string(),
            "declared coding resolution/failure_reason disagrees with recomputed test evidence",
        );
    }
    state
        .verified_artifacts
        .official_coding_tests
        .insert(run_path.to_string(), evidence);
}

fn validate_treatment_maintenance(
    run: &CodingRunArtifact,
    run_path: &str,
    label: &str,
    state: &mut VerifyState,
) {
    let Some(path) = artifact_file_path(run, "maintenance_evidence", label, state) else {
        return;
    };
    let Some(evidence) = read_json_artifact::<OfficialCodingMaintenanceEvidence>(
        &path,
        state,
        "official remem_e2e maintenance evidence",
    ) else {
        return;
    };
    let value = &evidence.value;
    let identity_valid = value.schema_version == 1
        && value.task_id == run.task_id
        && value.condition == "remem_e2e"
        && value.run_index == run.run_index
        && Some(value.attempt_id.as_str()) == run.attempt_id.as_deref()
        && !value.attempt_id.trim().is_empty();
    let measurement_valid = value.measurement.is_valid();
    if !identity_valid || !measurement_valid {
        state.fail(
            label.to_string(),
            "official remem_e2e maintenance evidence is unbound or internally inconsistent",
        );
        return;
    }
    state
        .verified_artifacts
        .treatment_maintenance
        .insert(run_path.to_string(), evidence);
}

fn validate_curator_evidence(
    run: &CodingRunArtifact,
    run_logical_path: &str,
    label: &str,
    state: &mut VerifyState,
) {
    let Some(log_path) = curator_artifact_path(run, "curator_log", label, state) else {
        return;
    };
    let Some(memory_path) = curator_artifact_path(run, "curated_memory", label, state) else {
        return;
    };
    let Some(log) = read_json_artifact::<CuratorLogArtifact>(
        &log_path,
        state,
        "curated_file_budgeted curator log",
    ) else {
        return;
    };
    let memory_bytes = match state.consume_file(&memory_path, "read curated MEMORY.md") {
        Ok(bytes) => bytes,
        Err(()) => return,
    };
    let Ok(memory_text) = std::str::from_utf8(&memory_bytes) else {
        state.fail(label.to_string(), "curated MEMORY.md must be UTF-8");
        return;
    };
    let value = &log.value;
    let mut valid = true;
    if value.schema_version != 1
        || value.condition != "curated_file_budgeted"
        || value.task_id != run.task_id
        || !value.target_blind
        || (value.budget.minutes_per_session - 3.0).abs() > f64::EPSILON
        || value.budget.max_chars != 4_000
        || value.sessions.is_empty()
    {
        state.fail(
            label.to_string(),
            "curator log identity or registered budget mismatch",
        );
        valid = false;
    }
    let mut episode_ids = BTreeSet::new();
    let mut minutes = 0.0;
    let mut updates = 0_u64;
    let mut deletions = 0_u64;
    let mut conflicts = 0_u64;
    for session in &value.sessions {
        if session.episode_id.trim().is_empty()
            || !episode_ids.insert(&session.episode_id)
            || !session.minutes_spent.is_finite()
            || session.minutes_spent < 0.0
            || session.minutes_spent > value.budget.minutes_per_session
            || session.chars_after > value.budget.max_chars
        {
            state.fail(
                label.to_string(),
                "curator log session identity or budget mismatch",
            );
            valid = false;
        }
        minutes += session.minutes_spent;
        updates = updates.saturating_add(session.edit_count);
        deletions = deletions.saturating_add(session.deletion_count);
        conflicts = conflicts.saturating_add(session.conflict_resolution_count);
    }
    if !value.totals.maintenance_minutes.is_finite()
        || (minutes - value.totals.maintenance_minutes).abs() > 1e-9
        || updates != value.totals.update_count
        || deletions != value.totals.deletion_count
        || conflicts != value.totals.conflict_resolution_count
    {
        state.fail(
            label.to_string(),
            "curator log totals do not match raw sessions",
        );
        valid = false;
    }
    let memory_sha256 = format!("{:x}", Sha256::digest(&memory_bytes));
    if memory_text.chars().count() != value.final_char_count
        || value.final_char_count > value.budget.max_chars
        || memory_sha256 != value.final_file_sha256
    {
        state.fail(
            label.to_string(),
            "curator log does not bind exact frozen MEMORY.md bytes",
        );
        valid = false;
    }
    if valid {
        state
            .verified_artifacts
            .curator_logs
            .insert(run_logical_path.to_string(), log);
    }
}

fn curator_artifact_path(
    run: &CodingRunArtifact,
    key: &str,
    label: &str,
    state: &mut VerifyState,
) -> Option<std::path::PathBuf> {
    let Some(raw_path) = run.artifacts.get(key) else {
        state.fail(
            label.to_string(),
            format!("curated_file_budgeted run requires {key} artifact"),
        );
        return None;
    };
    super::resolve_public_path(state, raw_path, raw_path)
}

fn validate_outcome(run: &CodingRunArtifact, label: &str, state: &mut VerifyState) {
    if run.resolved {
        if run.failure_reason.is_some() {
            state.fail(
                label.to_string(),
                "resolved coding run must not carry failure_reason",
            );
        }
    } else {
        let Some(reason) = run.failure_reason.as_deref().map(str::trim) else {
            state.fail(
                label.to_string(),
                "failed coding run must carry failure_reason",
            );
            return;
        };
        if !CODING_FAILURE_REASONS.contains(&reason) {
            state.fail(
                label.to_string(),
                "coding run has unknown failure_reason enum",
            );
        }
    }
}

fn validate_attempt_state(
    run: &CodingRunArtifact,
    raw_run: &Value,
    label: &str,
    state: &mut VerifyState,
) {
    let official = run.run_phase == "official";
    if official {
        for field in ["attempt_id", "target_started"] {
            if raw_run.get(field).is_none() {
                state.fail(
                    label.to_string(),
                    format!("official coding run must include explicit {field}"),
                );
            }
        }
    }

    match run.attempt_id.as_deref() {
        Some(attempt_id) if attempt_id.trim().is_empty() => {
            state.fail(label.to_string(), "coding run attempt_id must not be blank");
        }
        Some(attempt_id) if !state.coding_attempt_ids.insert(attempt_id.to_string()) => {
            state.fail(
                label.to_string(),
                "coding run attempt_id must be unique across the public artifact suite",
            );
        }
        Some(_) => {}
        None if official => {
            state.fail(
                label.to_string(),
                "official coding run attempt_id must be a non-blank string",
            );
        }
        None => {}
    }

    if official && run.target_started.is_none() {
        state.fail(
            label.to_string(),
            "official coding run target_started must be a boolean",
        );
    }
    if run.resolved && run.target_started == Some(false) {
        state.fail(
            label.to_string(),
            "resolved coding run cannot report target_started=false",
        );
    }
    if run.attempt_id.is_some() != run.target_started.is_some() {
        state.fail(
            label.to_string(),
            "coding run attempt_id and target_started must be present together",
        );
    }
}

fn validate_coding_metrics(run: &CodingRunArtifact, label: &str, state: &mut VerifyState) {
    if let (Some(input), Some(output), Some(total)) = (
        run.metrics.tokens_input,
        run.metrics.tokens_output,
        run.metrics.tokens_total,
    ) {
        if input.saturating_add(output) != total {
            state.fail(label.to_string(), "coding run token totals do not add up");
        }
    } else {
        state.fail(
            label.to_string(),
            "coding run must include complete token accounting",
        );
    }
    if run.metrics.turns.is_none() {
        state.fail(label.to_string(), "coding run is missing turns");
    }
    if run.metrics.wall_time_ms.is_none() {
        state.fail(label.to_string(), "coding run is missing wall_time_ms");
    }
    if run.metrics.tool_calls.is_none() {
        state.fail(label.to_string(), "coding run is missing tool_calls");
    }
    if run.metrics.commands_run.is_none() {
        state.fail(label.to_string(), "coding run is missing commands_run");
    }
}

fn validate_coding_context_audit(
    run: &CodingRunArtifact,
    raw_run: &Value,
    label: &str,
    state: &mut VerifyState,
) {
    if requires_context_audit(&run.condition) {
        require_explicit_audit_fields(raw_run, label, state);
        if run.context_audit_status != Some(RememContextAuditStatus::Verified) {
            state.fail(
                label.to_string(),
                "current remem-backed coding run must carry verified ContextAudit status",
            );
        }
        if run.context_audit_failure_reason.is_some() {
            state.fail(
                label.to_string(),
                "verified ContextAudit must not carry a failure reason",
            );
        }
        let Some(snapshot) = &run.remem_context_audit else {
            state.fail(
                label.to_string(),
                "current remem-backed coding run must include a ContextAudit snapshot",
            );
            return;
        };
        if let Err(error) = verify_context_audit_snapshot(snapshot) {
            state.fail(
                label.to_string(),
                format!("invalid ContextAudit snapshot: {error}"),
            );
        } else {
            validate_persisted_context_audit_provenance(run, snapshot, label, state);
        }
    } else if run.condition == "remem_preloaded" {
        if run.context_audit_status.is_some()
            || run.context_audit_failure_reason.is_some()
            || run.remem_context_audit.is_some()
            || run.injected_context_sha256.is_some()
        {
            state.fail(
                label.to_string(),
                "historical remem_preloaded run must not claim current ContextAudit evidence",
            );
        }
    } else {
        require_explicit_audit_fields(raw_run, label, state);
        if run.context_audit_status != Some(RememContextAuditStatus::NotApplicable) {
            state.fail(
                label.to_string(),
                "non-remem coding condition must mark ContextAudit not_applicable",
            );
        }
        if run.context_audit_failure_reason.is_some()
            || run.remem_context_audit.is_some()
            || run.injected_context_sha256.is_some()
        {
            state.fail(
                label.to_string(),
                "not_applicable ContextAudit must not carry a failure reason or snapshot",
            );
        }
    }
}

fn validate_persisted_context_audit_provenance(
    run: &CodingRunArtifact,
    snapshot: &crate::eval::coding_bench::RememContextAuditSnapshot,
    label: &str,
    state: &mut VerifyState,
) {
    let Some(context_path) = artifact_file_path(run, "injected_context", label, state) else {
        return;
    };
    let Some(database_path) = artifact_file_path(run, "remem_db_snapshot", label, state) else {
        return;
    };
    let injected_context = match state
        .consume_file(&context_path, "read injected_context artifact")
        .and_then(|bytes| String::from_utf8(bytes).map_err(|_| ()))
    {
        Ok(context) => context,
        Err(()) => {
            state.fail(label.to_string(), "injected_context artifact must be UTF-8");
            return;
        }
    };
    let actual_sha256 = format!("{:x}", Sha256::digest(injected_context.as_bytes()));
    if run.injected_context_sha256.as_deref() != Some(actual_sha256.as_str()) {
        state.fail(
            label.to_string(),
            "injected_context_sha256 must bind the exact injected_context bytes",
        );
        return;
    }
    let database_bytes = match state.consume_file(&database_path, "read remem_db_snapshot") {
        Ok(bytes) => bytes,
        Err(()) => return,
    };
    run_after_coding_snapshot_consumed_hook(&database_path);
    let connection = match super::security_snapshot::open_consumed_read_only_sqlite(&database_bytes)
    {
        Ok(connection) => connection,
        Err(error) => {
            state.fail(
                label.to_string(),
                format!("open consumed read-only remem_db_snapshot: {error:#}"),
            );
            return;
        }
    };
    if let Err(error) =
        verify_snapshot_against_persisted_injection(&connection, snapshot, &injected_context)
    {
        state.fail(
            label.to_string(),
            format!("ContextAudit provenance verification failed: {error}"),
        );
    }
}

fn artifact_file_path(
    run: &CodingRunArtifact,
    key: &str,
    label: &str,
    state: &mut VerifyState,
) -> Option<std::path::PathBuf> {
    let raw_path = run.artifacts.get(key)?;
    let path = super::resolve_public_path(state, raw_path, raw_path)?;
    if !path.is_file() {
        state.fail(
            label.to_string(),
            format!("artifact file for {key} is missing"),
        );
        return None;
    }
    Some(path)
}

fn require_explicit_audit_fields(raw_run: &Value, label: &str, state: &mut VerifyState) {
    for field in [
        "context_audit_status",
        "context_audit_failure_reason",
        "remem_context_audit",
        "injected_context_sha256",
    ] {
        if raw_run.get(field).is_none() {
            state.fail(
                label.to_string(),
                format!("coding run must include explicit {field}"),
            );
        }
    }
}

pub(super) fn is_public_coding_condition(condition: &str) -> bool {
    PUBLIC_CODING_CONDITIONS.contains(&condition)
}

#[cfg(test)]
pub(in crate::eval::bench_artifact) fn public_coding_conditions() -> &'static [&'static str] {
    &PUBLIC_CODING_CONDITIONS
}

pub(in crate::eval::bench_artifact) fn requires_remem_evidence(condition: &str) -> bool {
    condition.starts_with("remem_")
}

fn requires_context_audit(condition: &str) -> bool {
    requires_remem_evidence(condition) && condition != "remem_preloaded"
}

fn validate_coding_memory_contract(
    contract: &CodingMemoryContract,
    failure_reason: Option<&str>,
    label: &str,
    state: &mut VerifyState,
) {
    validate_rate(
        contract.citation_precision,
        "memory_contract.citation_precision",
        label,
        state,
    );
    validate_rate(
        contract.citation_recall,
        "memory_contract.citation_recall",
        label,
        state,
    );
    require_unique_positive_ids(
        &contract.injected_memory_ids,
        "memory_contract.injected_memory_ids",
        label,
        state,
    );
    require_unique_positive_ids(
        &contract.used_memory_ids,
        "memory_contract.used_memory_ids",
        label,
        state,
    );
    if contract.memory_helped && contract.memory_hurt {
        state.fail(
            label.to_string(),
            "memory_contract cannot mark both memory_helped and memory_hurt",
        );
    }
    let recomputed_memory_hurt = recompute_memory_hurt(failure_reason, contract.stale_used_count);
    if contract.memory_hurt != recomputed_memory_hurt {
        state.fail(
            label.to_string(),
            "memory_contract.memory_hurt must match recomputed memory harm from failure_reason and stale_used_count",
        );
    }
}

pub(in crate::eval::bench_artifact) fn recompute_memory_hurt(
    failure_reason: Option<&str>,
    stale_used_count: u64,
) -> bool {
    failure_reason.is_some_and(is_memory_specific_failure_reason) || stale_used_count > 0
}

fn validate_rate(value: f64, field: &str, label: &str, state: &mut VerifyState) {
    if !(0.0..=1.0).contains(&value) || !value.is_finite() {
        state.fail(
            label.to_string(),
            format!("{field} must be a finite rate between 0 and 1"),
        );
    }
}

fn require_unique_positive_ids(ids: &[i64], field: &str, label: &str, state: &mut VerifyState) {
    let mut seen = BTreeSet::new();
    for id in ids {
        if *id <= 0 {
            state.fail(
                label.to_string(),
                format!("{field} contains non-positive id"),
            );
        }
        if !seen.insert(*id) {
            state.fail(label.to_string(), format!("{field} contains duplicate id"));
        }
    }
}

fn is_memory_specific_failure_reason(reason: &str) -> bool {
    matches!(
        reason,
        "ignored_memory"
            | "missing_memory"
            | "stale_memory_followed"
            | "irrelevant_memory_distracted"
            | "agent_hallucinated_memory"
    )
}
