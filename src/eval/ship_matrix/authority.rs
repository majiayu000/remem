use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{ImplementationIdentity, ShipMatrixOptions};
use crate::eval::bench_artifact::PublicBaselineReport;

const CODING_CLAIM_IDS: [&str; 3] = [
    "remem-e2e-vs-no-memory-v1",
    "remem-e2e-vs-curated-file-budgeted-v1",
    "remem-e2e-stop-loss-v1",
];

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
    security: Option<&Value>,
    implementation: &ImplementationIdentity,
) -> SecurityAuthority {
    let mut diagnostics = Vec::new();
    if !report.is_some_and(|value| value.artifact_verifier.passed) {
        diagnostics.push("public artifact verifier did not pass".to_string());
    }
    verify_security_report_binding(options, report, &mut diagnostics);

    verify_security_outcomes(security, &mut diagnostics);
    let identities = security
        .and_then(|value| value.get("run_artifacts"))
        .and_then(Value::as_array)
        .map(|paths| collect_security_identities(&options.public_root, paths, &mut diagnostics))
        .unwrap_or_else(|| {
            diagnostics.push("security report has no run_artifacts array".to_string());
            SecurityRunIdentities::default()
        });
    verify_security_task_set(options, &identities.task_ids, &mut diagnostics);
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
            verify_security_suite_binding(options, security, commit, &identities, &mut diagnostics);
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
    let selected = match options.security_report_path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(format!(
                "cannot resolve selected security report {}: {error}",
                options.security_report_path.display()
            ));
            return;
        }
    };
    let covered = report.reports.iter().any(|entry| {
        entry.benchmark_id == "adversarial-policy"
            && entry.benchmark_version == "v2"
            && security_report_path_matches(options, &entry.path, &selected)
    });
    if !covered {
        diagnostics.push(format!(
            "verified public manifest does not cover selected adversarial-policy v2 report {}",
            options.security_report_path.display()
        ));
    }
}

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
    public_root: &Path,
    paths: &[Value],
    diagnostics: &mut Vec<String>,
) -> SecurityRunIdentities {
    let mut identities = SecurityRunIdentities::default();
    let root = match public_root.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            diagnostics.push(format!(
                "cannot resolve public root {}: {error}",
                public_root.display()
            ));
            return identities;
        }
    };
    if paths.is_empty() {
        diagnostics.push("security report run_artifacts is empty".to_string());
    }
    for value in paths {
        let Some(relative) = value.as_str() else {
            diagnostics.push("security run_artifacts contains a non-string path".to_string());
            continue;
        };
        let candidate = public_root.join(relative);
        let resolved = match candidate.canonicalize() {
            Ok(path) if path.starts_with(&root) => path,
            Ok(_) => {
                diagnostics.push(format!("security run path escapes public root: {relative}"));
                continue;
            }
            Err(error) => {
                diagnostics.push(format!("cannot resolve security run {relative}: {error}"));
                continue;
            }
        };
        let Some(run) = super::read_json_value(&resolved).ok() else {
            diagnostics.push(format!("security run is invalid JSON: {relative}"));
            continue;
        };
        match run.get("task_id").and_then(Value::as_str) {
            Some(task_id) if identities.task_ids.insert(task_id.to_string()) => {}
            Some(task_id) => {
                diagnostics.push(format!("security run task_id is duplicated: {task_id}"))
            }
            None => diagnostics.push(format!("security run lacks task_id: {relative}")),
        }
        match run
            .pointer("/environment/remem_commit")
            .and_then(Value::as_str)
        {
            Some(commit) => {
                identities.commits.insert(commit.to_string());
            }
            None => diagnostics.push(format!(
                "security run lacks environment.remem_commit: {relative}"
            )),
        }
        match run
            .pointer("/environment/fixture_revision")
            .and_then(Value::as_str)
        {
            Some(revision) => {
                identities.fixture_revisions.insert(revision.to_string());
            }
            None => diagnostics.push(format!(
                "security run lacks environment.fixture_revision: {relative}"
            )),
        }
        match run.get("suite_content_identity").and_then(Value::as_str) {
            Some(identity) => {
                identities
                    .suite_content_identities
                    .insert(identity.to_string());
            }
            None => diagnostics.push(format!(
                "security run lacks suite_content_identity: {relative}"
            )),
        }
        match (
            run.pointer("/environment/os").and_then(Value::as_str),
            run.pointer("/environment/arch").and_then(Value::as_str),
        ) {
            (Some(os), Some(arch)) if !os.trim().is_empty() && !arch.trim().is_empty() => {
                identities
                    .platforms
                    .insert((os.to_string(), arch.to_string()));
            }
            _ => diagnostics.push(format!(
                "security run lacks a complete environment.os/arch identity: {relative}"
            )),
        }
        if run
            .pointer("/environment/source_dirty")
            .and_then(Value::as_bool)
            != Some(false)
        {
            diagnostics.push(format!(
                "security run lacks clean-source attestation: {relative}"
            ));
        }
        match run
            .pointer("/environment/production_input_tree_sha256")
            .and_then(Value::as_str)
        {
            Some(tree) if is_sha256(tree) => {
                identities.production_trees.insert(tree.to_string());
            }
            _ => diagnostics.push(format!(
                "security run lacks a valid production input tree SHA-256: {relative}"
            )),
        }
        if run
            .pointer("/metrics/policy/policy_failure_count")
            .and_then(Value::as_u64)
            != Some(0)
        {
            diagnostics.push(format!("security run has a policy failure: {relative}"));
        }
    }
    identities
}

fn verify_security_suite_binding(
    options: &ShipMatrixOptions,
    security: Option<&Value>,
    benchmark_commit: &str,
    identities: &SecurityRunIdentities,
    diagnostics: &mut Vec<String>,
) {
    let suite_path = options
        .public_root
        .join("memory/suites/adversarial-policy/suite.json");
    let current_bytes = match fs::read(&suite_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            diagnostics.push(format!(
                "cannot read current adversarial-policy v2 suite {}: {error}",
                suite_path.display()
            ));
            return;
        }
    };
    let report_identity = security
        .and_then(|value| value.pointer("/aggregate_metrics/suite_content_identity"))
        .and_then(Value::as_str);
    verify_executed_suite_identity(
        &current_bytes,
        report_identity,
        &identities.suite_content_identities,
        diagnostics,
    );
    let repo_root = match crate::git_util::resolve_toplevel(Path::new(".")) {
        Some(path) => path,
        None => {
            diagnostics.push("cannot resolve repository root for security suite".to_string());
            return;
        }
    };
    let suite_path = match suite_path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(format!("cannot resolve security suite path: {error}"));
            return;
        }
    };
    let relative = match suite_path.strip_prefix(&repo_root) {
        Ok(path) => path,
        Err(_) => {
            diagnostics.push(format!(
                "security suite {} is outside repository root {}",
                suite_path.display(),
                repo_root.display()
            ));
            return;
        }
    };
    let object = format!("{benchmark_commit}:{}", relative.to_string_lossy());
    let benchmark_bytes = match crate::git_util::git_output_soft(&repo_root, &["show", &object]) {
        Some(output) if output.status.success() => output.stdout,
        _ => {
            diagnostics.push(format!(
                "security benchmark commit {benchmark_commit} does not expose {}",
                relative.display()
            ));
            return;
        }
    };
    verify_security_suite_bytes(&current_bytes, &benchmark_bytes, diagnostics);

    let expected_revision = super::read_json_value(&suite_path).ok().and_then(|suite| {
        suite
            .get("fixture_revision")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let expected_revisions = expected_revision.into_iter().collect::<BTreeSet<_>>();
    if identities.fixture_revisions != expected_revisions {
        diagnostics.push(format!(
            "security run fixture revisions do not exactly bind the current suite: expected={expected_revisions:?} actual={:?}",
            identities.fixture_revisions
        ));
    }
}

pub(super) fn verify_executed_suite_identity(
    current_bytes: &[u8],
    report_identity: Option<&str>,
    run_identities: &BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) {
    let expected = suite_content_identity(current_bytes);
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

fn suite_content_identity(bytes: &[u8]) -> String {
    format!("sha256-raw-suite-v1:{:x}", Sha256::digest(bytes))
}

pub(super) fn verify_security_suite_bytes(
    current_bytes: &[u8],
    benchmark_bytes: &[u8],
    diagnostics: &mut Vec<String>,
) {
    let current = format!("{:x}", Sha256::digest(current_bytes));
    let benchmark = format!("{:x}", Sha256::digest(benchmark_bytes));
    if current != benchmark {
        diagnostics.push(format!(
            "adversarial-policy v2 suite content identity changed: benchmark={benchmark} current={current}"
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

fn security_source_equivalent(
    benchmark_commit: &str,
    benchmark_tree: Option<&str>,
    implementation: &ImplementationIdentity,
    diagnostics: &mut Vec<String>,
) -> bool {
    let Some(current) = implementation.git_sha.as_deref() else {
        diagnostics.push("current implementation Git SHA is unavailable".to_string());
        return false;
    };
    if !git_succeeds(&["merge-base", "--is-ancestor", benchmark_commit, current]) {
        diagnostics.push(format!(
            "security benchmark commit {benchmark_commit} is not an ancestor of {current}"
        ));
        return false;
    }
    let current_tree = production_input_tree_sha256();
    if benchmark_tree.is_none() || current_tree.as_deref() != benchmark_tree {
        diagnostics.push(format!(
            "production input tree differs: benchmark={} current={}",
            benchmark_tree.unwrap_or("unavailable"),
            current_tree.as_deref().unwrap_or("unavailable")
        ));
        return false;
    }
    let production_pathspec = [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        ".cargo",
        "rust-toolchain.toml",
        "src",
        "prompts",
        "assets",
        ":(exclude)src/eval/ship_matrix.rs",
        ":(exclude)src/eval/ship_matrix/**",
        ":(exclude)src/eval/gates.rs",
    ];
    let mut committed_args = vec!["diff", "--quiet", benchmark_commit, current, "--"];
    committed_args.extend(production_pathspec);
    if !git_succeeds(&committed_args) {
        diagnostics.push(format!(
            "production source changed after security benchmark commit {benchmark_commit}"
        ));
        return false;
    }
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

fn production_input_tree_sha256() -> Option<String> {
    let output = crate::git_util::git_output_soft(
        Path::new("."),
        &[
            "ls-files",
            "-s",
            "--",
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            ".cargo",
            "rust-toolchain.toml",
            "src",
            "prompts",
            "assets",
            ":(exclude)src/eval/ship_matrix.rs",
            ":(exclude)src/eval/ship_matrix/**",
            ":(exclude)src/eval/gates.rs",
        ],
    )?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(&output.stdout)))
}

fn verify_security_outcomes(security: Option<&Value>, diagnostics: &mut Vec<String>) {
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
            .and_then(|value| value.pointer(pointer))
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
    let mut claims_passed = true;
    for id in CODING_CLAIM_IDS {
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
            id,
            &repo_root,
            implementation.git_sha.as_deref(),
            &mut diagnostics,
        );
    }
    diagnostics.push(
        "claim registry schema has no independently verified Level 3 claim authority".to_string(),
    );
    ClaimAuthority {
        coding_passed: schema_valid && locked && claims_passed,
        level3_passed: false,
        diagnostics,
    }
}

fn verify_pass_claim(
    claim: &Value,
    id: &str,
    repo_root: &Path,
    implementation_sha: Option<&str>,
    diagnostics: &mut Vec<String>,
) -> bool {
    if claim.get("status").and_then(Value::as_str) != Some("PASS") {
        diagnostics.push(format!("claim {id} is not PASS"));
        return false;
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
    match fs::read(repo_root.join(relative)) {
        Ok(bytes) if format!("{:x}", Sha256::digest(&bytes)) == expected => {
            supporting_report_binds_implementation(&bytes, id, implementation_sha, diagnostics)
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
    let bound = report
        .pointer("/reproducibility/remem_commits")
        .and_then(Value::as_array)
        .is_some_and(|commits| {
            commits
                .iter()
                .any(|commit| commit.as_str() == Some(implementation_sha))
        });
    if !bound {
        diagnostics.push(format!(
            "claim {id} supporting_report does not bind current implementation SHA {implementation_sha}"
        ));
    }
    bound
}
