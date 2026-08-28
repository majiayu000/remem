use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{ImplementationIdentity, ShipMatrixOptions};
use crate::eval::bench_artifact::{
    MemoryRunArtifact, PublicBaselineReport, PublicBenchmarkReport, VerifiedArtifact,
};
use crate::eval::memory_bench::types::MemoryBenchSuiteFixture;

pub(super) struct SecurityAuthority {
    pub(super) passed: bool,
    pub(super) benchmark_commit: Option<String>,
    pub(super) diagnostics: Vec<String>,
}

pub(super) struct ClaimAuthority {
    pub(super) coding_passed: bool,
    pub(super) level3_passed: bool,
    pub(super) diagnostics: Vec<String>,
}

pub(super) fn verify_security_authority(
    options: &ShipMatrixOptions,
    report: Option<&PublicBaselineReport>,
    security: Option<&PublicBenchmarkReport>,
    verified_memory_runs: &[VerifiedArtifact<MemoryRunArtifact>],
    verified_security_suite: Option<&VerifiedArtifact<MemoryBenchSuiteFixture>>,
    implementation: &ImplementationIdentity,
) -> SecurityAuthority {
    let mut diagnostics = Vec::new();
    if !implementation.executable_source_equivalent {
        diagnostics
            .push("executing eval binary does not match the checkout source identity".to_string());
    }
    if implementation.build_source_dirty != Some(false) {
        diagnostics.push("executing eval binary was built from dirty authority inputs".to_string());
    }
    if implementation.source_dirty != Some(false) {
        diagnostics.push("current checkout is not clean for security evaluation".to_string());
    }
    if !report.is_some_and(|value| value.artifact_verifier.passed) {
        diagnostics.push("public artifact verifier did not pass".to_string());
    }
    verify_security_report_binding(options, report, &mut diagnostics);

    verify_security_outcomes(security, &mut diagnostics);
    let identities = security
        .map(|value| {
            collect_security_identities(
                &value.run_artifacts,
                verified_memory_runs,
                &mut diagnostics,
            )
        })
        .unwrap_or_else(|| {
            diagnostics.push("security report has no run_artifacts array".to_string());
            SecurityRunIdentities::default()
        });
    verify_typed_security_task_set(
        verified_security_suite,
        &identities.task_ids,
        &mut diagnostics,
    );
    let benchmark_commit = if identities.commits.len() == 1 {
        identities.commits.first().cloned()
    } else {
        diagnostics.push(format!(
            "security runs must bind exactly one implementation commit; found {}",
            identities.commits.len()
        ));
        None
    };
    let benchmark_tree = if identities.production_trees.len() == 1 {
        identities.production_trees.first().cloned()
    } else {
        diagnostics.push(format!(
            "security runs must bind exactly one production input tree; found {}",
            identities.production_trees.len()
        ));
        None
    };
    verify_security_platforms(
        &identities.platforms,
        implementation.os,
        implementation.arch,
        &mut diagnostics,
    );

    if let Some(commit) = benchmark_commit.as_deref() {
        if !is_full_git_sha(commit) {
            diagnostics.push(format!(
                "security run commit is not a full Git SHA: {commit}"
            ));
        } else {
            verify_security_suite_binding(
                security,
                verified_security_suite,
                &identities,
                &mut diagnostics,
            );
            if !security_source_equivalent(
                commit,
                benchmark_tree.as_deref(),
                implementation,
                &mut diagnostics,
            ) {
                diagnostics.push(
                    "security artifact is stale for the current production source tree".to_string(),
                );
            }
        }
    }

    SecurityAuthority {
        passed: diagnostics.is_empty(),
        benchmark_commit,
        diagnostics,
    }
}

#[derive(Default)]
struct SecurityRunIdentities {
    commits: BTreeSet<String>,
    production_trees: BTreeSet<String>,
    task_ids: BTreeSet<String>,
    fixture_revisions: BTreeSet<String>,
    suite_content_identities: BTreeSet<String>,
    platforms: BTreeSet<(String, String)>,
}

fn verify_security_report_binding(
    options: &ShipMatrixOptions,
    report: Option<&PublicBaselineReport>,
    diagnostics: &mut Vec<String>,
) {
    let Some(report) = report else {
        diagnostics.push("verified public manifest omitted adversarial-policy v2".to_string());
        return;
    };
    let selected = options
        .security_report_path
        .strip_prefix(&options.public_root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"));
    let covered = report.reports.iter().any(|entry| {
        entry.benchmark_id == "adversarial-policy"
            && entry.benchmark_version == "v2"
            && selected.as_deref() == Some(entry.path.as_str())
    });
    if !covered {
        diagnostics.push(format!(
            "verified public manifest does not cover selected adversarial-policy v2 report {}",
            options.security_report_path.display()
        ));
    }
}

#[cfg(test)]
pub(super) fn security_report_path_matches(
    options: &ShipMatrixOptions,
    manifest_entry_path: &str,
    selected: &Path,
) -> bool {
    options
        .public_root
        .join(manifest_entry_path)
        .canonicalize()
        .is_ok_and(|path| path == selected)
}

fn collect_security_identities(
    paths: &[String],
    verified_runs: &[VerifiedArtifact<MemoryRunArtifact>],
    diagnostics: &mut Vec<String>,
) -> SecurityRunIdentities {
    let mut identities = SecurityRunIdentities::default();
    if paths.is_empty() {
        diagnostics.push("security report run_artifacts is empty".to_string());
    }
    for relative in paths {
        let Some(run) = verified_runs
            .iter()
            .find(|artifact| artifact.path == *relative)
            .map(|artifact| &artifact.value)
        else {
            diagnostics.push(format!(
                "security run is absent from verifier snapshot: {relative}"
            ));
            continue;
        };
        if !identities.task_ids.insert(run.task_id.clone()) {
            diagnostics.push(format!(
                "security run task_id is duplicated: {}",
                run.task_id
            ));
        }
        if run.environment.remem_commit.trim().is_empty() {
            diagnostics.push(format!(
                "security run lacks environment.remem_commit: {relative}"
            ));
        } else {
            identities
                .commits
                .insert(run.environment.remem_commit.clone());
        }
        match run.environment.fixture_revision.as_deref() {
            Some(revision) => {
                identities.fixture_revisions.insert(revision.to_string());
            }
            None => diagnostics.push(format!(
                "security run lacks environment.fixture_revision: {relative}"
            )),
        }
        match run.suite_content_identity.as_deref() {
            Some(identity) => {
                identities
                    .suite_content_identities
                    .insert(identity.to_string());
            }
            None => diagnostics.push(format!(
                "security run lacks suite_content_identity: {relative}"
            )),
        }
        match (&run.environment.os, &run.environment.arch) {
            (os, arch) if !os.trim().is_empty() && !arch.trim().is_empty() => {
                identities
                    .platforms
                    .insert((os.to_string(), arch.to_string()));
            }
            _ => diagnostics.push(format!(
                "security run lacks a complete environment.os/arch identity: {relative}"
            )),
        }
        if run.environment.source_dirty != Some(false) {
            diagnostics.push(format!(
                "security run lacks clean-source attestation: {relative}"
            ));
        }
        match run.environment.production_input_tree_sha256.as_deref() {
            Some(tree) if is_sha256(tree) => {
                identities.production_trees.insert(tree.to_string());
            }
            _ => diagnostics.push(format!(
                "security run lacks a valid production input tree SHA-256: {relative}"
            )),
        }
        if run
            .metrics
            .pointer("/policy/policy_failure_count")
            .and_then(Value::as_u64)
            != Some(0)
        {
            diagnostics.push(format!("security run has a policy failure: {relative}"));
        }
    }
    identities
}

fn verify_security_suite_binding(
    security: Option<&PublicBenchmarkReport>,
    suite: Option<&VerifiedArtifact<MemoryBenchSuiteFixture>>,
    identities: &SecurityRunIdentities,
    diagnostics: &mut Vec<String>,
) {
    let Some(suite) = suite else {
        diagnostics.push("verified snapshot omitted adversarial-policy v2 suite".to_string());
        return;
    };
    let report_identity = security
        .and_then(|value| value.aggregate_metrics.pointer("/suite_content_identity"))
        .and_then(Value::as_str);
    verify_executed_suite_digest(
        &suite.sha256,
        report_identity,
        &identities.suite_content_identities,
        diagnostics,
    );
    let expected_revisions = BTreeSet::from([suite.value.fixture_revision.clone()]);
    if identities.fixture_revisions != expected_revisions {
        diagnostics.push(format!(
            "security run fixture revisions do not exactly bind the current suite: expected={expected_revisions:?} actual={:?}",
            identities.fixture_revisions
        ));
    }
}

#[cfg(test)]
pub(super) fn verify_executed_suite_identity(
    current_bytes: &[u8],
    report_identity: Option<&str>,
    run_identities: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) {
    verify_executed_suite_digest(
        &format!("{:x}", Sha256::digest(current_bytes)),
        report_identity,
        run_identities,
        diagnostics,
    );
}

fn verify_executed_suite_digest(
    digest: &str,
    report_identity: Option<&str>,
    run_identities: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) {
    let expected = format!("sha256-raw-suite-v1:{digest}");
    if report_identity != Some(expected.as_str()) {
        diagnostics.push(format!(
            "security report suite content identity mismatch: expected={expected} actual={}",
            report_identity.unwrap_or("unavailable")
        ));
    }
    let expected_runs = BTreeSet::from([expected.clone()]);
    if run_identities != &expected_runs {
        diagnostics.push(format!(
            "security runs do not exactly bind the executed suite content: expected={expected_runs:?} actual={run_identities:?}"
        ));
    }
}

pub(super) fn verify_security_platforms(
    actual: &BTreeSet<(String, String)>,
    os: &str,
    arch: &str,
    diagnostics: &mut Vec<String>,
) {
    if actual != &BTreeSet::from([(os.to_string(), arch.to_string())]) {
        diagnostics.push(format!(
            "security report does not exactly cover the evaluated platform {os}/{arch}; found {actual:?}"
        ));
    }
}

#[cfg(test)]
pub(super) fn verify_security_task_set(
    options: &ShipMatrixOptions,
    actual: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) {
    let suite_path = options
        .public_root
        .join("memory/suites/adversarial-policy/suite.json");
    let suite = match super::read_json_value(&suite_path) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(format!("cannot load adversarial-policy v2 suite: {error}"));
            return;
        }
    };
    if suite.get("version").and_then(Value::as_str) != Some("v2")
        || suite.get("fixture_revision").and_then(Value::as_str) != Some("adversarial-policy-v2")
    {
        diagnostics.push(
            "adversarial-policy suite identity must be version v2 and fixture_revision adversarial-policy-v2"
                .to_string(),
        );
    }
    let mut expected = BTreeSet::new();
    let Some(tasks) = suite.get("tasks").and_then(Value::as_array) else {
        diagnostics.push("adversarial-policy v2 suite has no tasks array".to_string());
        return;
    };
    for task in tasks {
        match task.get("id").and_then(Value::as_str) {
            Some(id) if expected.insert(id.to_string()) => {}
            Some(id) => diagnostics.push(format!(
                "adversarial-policy v2 suite task_id is duplicated: {id}"
            )),
            None => diagnostics
                .push("adversarial-policy v2 suite contains a task without an id".to_string()),
        }
    }
    if expected.is_empty() {
        diagnostics.push("adversarial-policy v2 suite task set is empty".to_string());
        return;
    }
    let missing = expected.difference(actual).cloned().collect::<Vec<_>>();
    let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
    if !missing.is_empty() || !unexpected.is_empty() {
        diagnostics.push(format!(
            "security run task set mismatch: missing=[{}] unexpected=[{}]",
            missing.join(","),
            unexpected.join(",")
        ));
    }
}

fn verify_typed_security_task_set(
    suite: Option<&VerifiedArtifact<MemoryBenchSuiteFixture>>,
    actual: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) {
    let Some(suite) = suite else {
        diagnostics.push("verified snapshot omitted adversarial-policy v2 suite".to_string());
        return;
    };
    if suite.value.version != "v2" || suite.value.fixture_revision != "adversarial-policy-v2" {
        diagnostics.push(
            "adversarial-policy suite identity must be version v2 and fixture_revision adversarial-policy-v2"
                .to_string(),
        );
    }
    let mut expected = BTreeSet::new();
    for task in &suite.value.tasks {
        if !expected.insert(task.id.clone()) {
            diagnostics.push(format!(
                "adversarial-policy v2 suite task_id is duplicated: {}",
                task.id
            ));
        }
    }
    if expected.is_empty() {
        diagnostics.push("adversarial-policy v2 suite task set is empty".to_string());
        return;
    }
    let missing = expected.difference(actual).cloned().collect::<Vec<_>>();
    let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
    if !missing.is_empty() || !unexpected.is_empty() {
        diagnostics.push(format!(
            "security run task set mismatch: missing=[{}] unexpected=[{}]",
            missing.join(","),
            unexpected.join(",")
        ));
    }
}

fn security_source_equivalent(
    _benchmark_commit: &str,
    benchmark_tree: Option<&str>,
    _implementation: &ImplementationIdentity,
    diagnostics: &mut Vec<String>,
) -> bool {
    let current_tree = production_input_tree_sha256();
    if benchmark_tree.is_none() || current_tree.as_deref() != benchmark_tree {
        diagnostics.push(format!(
            "production input tree differs: benchmark={} current={}",
            benchmark_tree.unwrap_or("unavailable"),
            current_tree.as_deref().unwrap_or("unavailable")
        ));
        return false;
    }
    let production_pathspec = production_pathspec();
    let mut worktree_args = vec!["diff", "--quiet", "--"];
    worktree_args.extend(production_pathspec);
    let mut index_args = vec!["diff", "--cached", "--quiet", "--"];
    index_args.extend(production_pathspec);
    if !git_succeeds(&worktree_args) || !git_succeeds(&index_args) {
        diagnostics
            .push("uncommitted production-source changes invalidate security evidence".to_string());
        return false;
    }
    true
}

fn git_succeeds(args: &[&str]) -> bool {
    crate::git_util::git_output_soft(Path::new("."), args)
        .is_some_and(|output| output.status.success())
}

fn is_full_git_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn production_input_tree_sha256() -> Option<String> {
    let pathspec = production_pathspec();
    let mut args = vec!["ls-files", "-s", "--"];
    args.extend(pathspec);
    let output = crate::git_util::git_output_soft(Path::new("."), &args)?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(&output.stdout)))
}

fn production_pathspec() -> [&'static str; 9] {
    [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        ".cargo",
        "rust-toolchain.toml",
        "src",
        "prompts",
        "assets",
        "eval/public/memory/suites/adversarial-policy/suite.json",
    ]
}

fn verify_security_outcomes(
    security: Option<&PublicBenchmarkReport>,
    diagnostics: &mut Vec<String>,
) {
    let required = [
        ("/aggregate_metrics/policy/non_retention_leak_rate", 0.0),
        ("/aggregate_metrics/policy/policy_failure_rate", 0.0),
        ("/aggregate_metrics/policy/false_block_rate", 0.0),
        ("/aggregate_metrics/policy/suppression_obeyed_rate", 1.0),
        (
            "/aggregate_metrics/policy/sensitive_restricted_default_exclusion_rate",
            1.0,
        ),
        ("/aggregate_metrics/policy/policy_abstention_accuracy", 1.0),
    ];
    for (pointer, expected) in required {
        let actual = security
            .and_then(|value| {
                value
                    .aggregate_metrics
                    .pointer(pointer.trim_start_matches("/aggregate_metrics"))
            })
            .and_then(Value::as_f64);
        if actual != Some(expected) {
            diagnostics.push(format!(
                "security stop-loss failed at {pointer}: expected {expected}, got {}",
                actual.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
            ));
        }
    }
}

pub(super) fn verify_claim_authority(
    path: &Path,
    contract_path: &Path,
    implementation: &ImplementationIdentity,
) -> ClaimAuthority {
    let mut diagnostics = Vec::new();
    let registry = match super::read_json_value(path) {
        Ok(value) => value,
        Err(error) => {
            return ClaimAuthority {
                coding_passed: false,
                level3_passed: false,
                diagnostics: vec![error],
            };
        }
    };
    let schema_valid = registry.get("schema_version").and_then(Value::as_u64) == Some(1);
    if !schema_valid {
        diagnostics.push("claim registry schema_version must be 1".to_string());
    }
    let locked = registry.get("locked").and_then(Value::as_bool) == Some(true);
    if !locked {
        diagnostics.push("claim registry is not locked".to_string());
    }
    let contract = match super::read_json_value(contract_path) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(error);
            Value::Null
        }
    };
    let contract_valid = contract.get("schema_version").and_then(Value::as_u64) == Some(1)
        && contract.get("closed").and_then(Value::as_bool) == Some(true)
        && contract.get("contract_id").and_then(Value::as_str) == Some("gh931-coding-claims-v1");
    if !contract_valid {
        diagnostics.push("coding claim contract is not closed schema v1".to_string());
    }
    let claims = registry
        .get("claims")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            diagnostics.push("claim registry has no claims array".to_string());
            Vec::new()
        });
    let repo_root =
        crate::git_util::resolve_toplevel(Path::new(".")).unwrap_or_else(|| PathBuf::from("."));
    let expected_claims = contract
        .get("claims")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut claims_passed = expected_claims.len() == claims.len() && !expected_claims.is_empty();
    if expected_claims.len() != claims.len() {
        diagnostics.push("claim registry does not exactly match the closed claim set".to_string());
    }
    for expected_claim in &expected_claims {
        let Some(id) = expected_claim.get("id").and_then(Value::as_str) else {
            diagnostics.push("coding claim contract contains a claim without id".to_string());
            claims_passed = false;
            continue;
        };
        let Some(claim) = claims
            .iter()
            .find(|claim| claim.get("id").and_then(Value::as_str) == Some(id))
        else {
            diagnostics.push(format!("claim registry is missing required claim {id}"));
            claims_passed = false;
            continue;
        };
        claims_passed &= verify_pass_claim(
            claim,
            expected_claim,
            id,
            &repo_root,
            implementation.git_sha.as_deref(),
            implementation.production_input_tree_sha256.as_deref(),
            &mut diagnostics,
        );
    }
    diagnostics.push(
        "claim registry schema has no independently verified Level 3 claim authority".to_string(),
    );
    ClaimAuthority {
        coding_passed: schema_valid
            && locked
            && contract_valid
            && claims_passed
            && implementation.executable_source_equivalent
            && implementation.build_source_dirty == Some(false)
            && implementation.source_dirty == Some(false),
        level3_passed: false,
        diagnostics,
    }
}

fn verify_pass_claim(
    claim: &Value,
    expected_claim: &Value,
    id: &str,
    repo_root: &Path,
    implementation_sha: Option<&str>,
    implementation_tree: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> bool {
    if claim.get("status").and_then(Value::as_str) != Some("PASS") {
        diagnostics.push(format!("claim {id} is not PASS"));
        return false;
    }
    for field in [
        "comparison",
        "metric",
        "allowed_wording",
        "forbidden_wording",
    ] {
        if claim.get(field) != expected_claim.get(field) {
            diagnostics.push(format!(
                "claim {id} does not match closed contract field {field}"
            ));
            return false;
        }
    }
    let Some(report) = claim.get("supporting_report") else {
        diagnostics.push(format!("claim {id} has no supporting_report"));
        return false;
    };
    let Some(relative) = report.get("path").and_then(Value::as_str) else {
        diagnostics.push(format!("claim {id} supporting_report has no path"));
        return false;
    };
    let Some(expected) = report.get("sha256").and_then(Value::as_str) else {
        diagnostics.push(format!("claim {id} supporting_report has no sha256"));
        return false;
    };
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        || !relative.starts_with("eval/public/")
    {
        diagnostics.push(format!(
            "claim {id} supporting_report path is not repository-relative public evidence"
        ));
        return false;
    }
    if !is_sha256(expected) || expected.bytes().any(|byte| byte.is_ascii_uppercase()) {
        diagnostics.push(format!(
            "claim {id} supporting_report sha256 is not lowercase hex"
        ));
        return false;
    }
    match fs::read(repo_root.join(relative_path)) {
        Ok(bytes) if format!("{:x}", Sha256::digest(&bytes)) == expected => {
            supporting_report_binds_implementation(
                &bytes,
                id,
                implementation_sha,
                implementation_tree,
                diagnostics,
            )
        }
        Ok(_) => {
            diagnostics.push(format!("claim {id} supporting_report sha256 mismatch"));
            false
        }
        Err(error) => {
            diagnostics.push(format!(
                "claim {id} supporting_report is unreadable: {error}"
            ));
            false
        }
    }
}

pub(super) fn supporting_report_binds_implementation(
    bytes: &[u8],
    id: &str,
    implementation_sha: Option<&str>,
    implementation_tree: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> bool {
    let Some(implementation_sha) = implementation_sha else {
        diagnostics.push(format!(
            "claim {id} cannot bind an unidentified current implementation"
        ));
        return false;
    };
    let report: Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(error) => {
            diagnostics.push(format!(
                "claim {id} supporting_report is invalid JSON: {error}"
            ));
            return false;
        }
    };
    let revision_bound = report
        .pointer("/reproducibility/remem_commits")
        .and_then(Value::as_array)
        .is_some_and(|commits| {
            !commits.is_empty()
                && commits
                    .iter()
                    .all(|commit| commit.as_str().is_some_and(is_full_git_sha))
        });
    if !revision_bound {
        diagnostics.push(format!(
            "claim {id} supporting_report lacks valid producing revisions"
        ));
    }
    let source_bound = report
        .pointer("/reproducibility/production_input_tree_sha256")
        .and_then(Value::as_str)
        .zip(implementation_tree)
        .is_some_and(|(expected, current)| expected == current);
    if !source_bound {
        diagnostics.push(format!(
            "claim {id} supporting_report production source is not equivalent to executable {implementation_sha}"
        ));
    }
    revision_bound && source_bound
}
