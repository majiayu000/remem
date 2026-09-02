use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::types::{
    AuthorityStatus, AuthorityVerdict, BenchVerifyFailure, ReleaseAuthorityVerdict,
    SecurityAuthorityVerdict, SecurityReportAuthorityVerdict, VerifiedBenchmarkArtifacts,
};

pub(in crate::eval::bench_artifact) mod gh931;
pub(in crate::eval::bench_artifact) mod implementation;

const REQUIRED_RELEASE_TARGETS: [&str; 4] = [
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];
const REGISTERED_ADVERSARIAL_SUITE_SHA256: &str =
    "56dad240cc175fb3d3900875f05351b9541f9ef845aa54eddfc460151f3e257d";

pub(super) struct AuthorityEvaluation {
    pub verdict: AuthorityVerdict,
    pub failures: Vec<BenchVerifyFailure>,
}

pub(super) fn evaluate(
    verified: &VerifiedBenchmarkArtifacts,
    consumed_bytes: &BTreeMap<String, String>,
    existing_failures: &[BenchVerifyFailure],
) -> AuthorityEvaluation {
    let (security, failures) = evaluate_security(verified);
    let implementation = implementation::runtime_binding();
    let gh931 = gh931::evaluate(verified, existing_failures, &implementation);
    let release = evaluate_release(verified, &security, &implementation);
    let failed = !existing_failures.is_empty()
        || !failures.is_empty()
        || security.status == AuthorityStatus::Fail
        || gh931.status == AuthorityStatus::Fail;
    let status = if failed {
        AuthorityStatus::Fail
    } else if release.ready {
        AuthorityStatus::Pass
    } else {
        AuthorityStatus::Insufficient
    };
    let mut diagnostics = Vec::new();
    if !release.ready {
        diagnostics.push("release evidence is incomplete for the closed target set".to_string());
    }
    AuthorityEvaluation {
        verdict: AuthorityVerdict {
            schema_version: 1,
            status,
            consumed_bytes: consumed_bytes.clone(),
            implementation,
            security,
            gh931,
            release,
            diagnostics,
        },
        failures,
    }
}

fn evaluate_security(
    verified: &VerifiedBenchmarkArtifacts,
) -> (SecurityAuthorityVerdict, Vec<BenchVerifyFailure>) {
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for report in verified.reports.iter().filter(|report| {
        report.value.benchmark_id == "adversarial-policy" && report.value.benchmark_version == "v2"
    }) {
        let mut diagnostics = Vec::new();
        let report_runs = report
            .value
            .run_artifacts
            .iter()
            .filter_map(|path| {
                let run = verified.memory_runs.iter().find(|run| run.path == *path);
                if run.is_none() {
                    diagnostics.push(format!("missing verified security run {path}"));
                }
                run
            })
            .collect::<Vec<_>>();
        for message in security_report_coverage_diagnostics(report, &report_runs, verified) {
            diagnostics.push(message.clone());
            failures.push(BenchVerifyFailure {
                path: report.path.clone(),
                message,
            });
        }
        let outcomes = report
            .value
            .run_artifacts
            .iter()
            .filter_map(|path| {
                let outcome = verified.security_policy_outcomes.get(path);
                if outcome.is_none() {
                    diagnostics.push(format!("missing recomputed security policy for {path}"));
                }
                outcome.cloned()
            })
            .collect::<Vec<_>>();
        let complete = outcomes.len() == report.value.run_artifacts.len()
            && report_runs.len() == report.value.run_artifacts.len();
        let summary = complete
            .then(|| crate::eval::memory_bench::summarize_verified_security_policy(&outcomes));
        if let Some(summary) = summary.as_ref() {
            let suite = verified.memory_suites.iter().find(|suite| {
                suite.value.benchmark_id == report.value.benchmark_id
                    && suite.value.version == report.value.benchmark_version
                    && report.value.suite.as_deref() == Some(suite.value.suite.as_str())
            });
            let recomputed = suite.map(|suite| {
                serde_json::json!({
                    "suite": suite.value.suite,
                    "suite_version": suite.value.version,
                    "fixture_revision": suite.value.fixture_revision,
                    "suite_content_identity": format!("sha256-raw-suite-v1:{}", suite.sha256),
                    "run_count": outcomes.len(),
                    "policy": summary,
                    "verification_paths": outcomes.iter()
                        .map(|outcome| outcome.verification_path.clone())
                        .collect::<BTreeSet<_>>(),
                    "measurement_sources": outcomes.iter()
                        .map(|outcome| outcome.measurement_source.clone())
                        .collect::<BTreeSet<_>>(),
                })
            });
            if recomputed.as_ref() != Some(&report.value.aggregate_metrics) {
                let message = format!(
                    "security report must exactly match the exact recomputed security aggregate: declared={} recomputed={}",
                    report.value.aggregate_metrics,
                    recomputed.as_ref().unwrap_or(&Value::Null)
                );
                diagnostics.push(message.clone());
                failures.push(BenchVerifyFailure {
                    path: report.path.clone(),
                    message,
                });
            }
        }
        let policy_failure_count = outcomes
            .iter()
            .map(|outcome| outcome.policy_failure_count)
            .sum();
        if policy_failure_count > 0 {
            let message = format!(
                "recomputed security policy stop-loss failed in {policy_failure_count} outcome(s)"
            );
            diagnostics.push(message.clone());
            failures.push(BenchVerifyFailure {
                path: report.path.clone(),
                message,
            });
        }
        let binding = security_report_binding(&report_runs);
        if !binding.commit_tree_verified {
            let message = "security report producing commit must resolve to its attested production-input tree".to_string();
            diagnostics.push(message.clone());
            failures.push(BenchVerifyFailure {
                path: report.path.clone(),
                message,
            });
        }
        if !binding.model_execution_identity_consistent {
            let message = "security report runs must share one model execution identity after excluding prompt_hash".to_string();
            diagnostics.push(message.clone());
            failures.push(BenchVerifyFailure {
                path: report.path.clone(),
                message,
            });
        }
        if !binding.ready {
            let message = "security report run identity binding is incomplete".to_string();
            diagnostics.push(message.clone());
            failures.push(BenchVerifyFailure {
                path: report.path.clone(),
                message,
            });
        }
        let status = if complete && diagnostics.is_empty() {
            AuthorityStatus::Pass
        } else {
            AuthorityStatus::Fail
        };
        reports.push(SecurityReportAuthorityVerdict {
            report_path: report.path.clone(),
            report_sha256: report.sha256.clone(),
            status,
            target: binding.target,
            models: binding.models,
            platforms: binding.platforms,
            producing_shas: binding.producing_shas,
            production_input_trees: binding.production_input_trees,
            source_dirty_attestations: binding.source_dirty_attestations,
            runs_recomputed: outcomes.len(),
            policy_failure_count,
            policy_summary: summary,
            diagnostics,
        });
    }
    let status = if reports.is_empty() {
        AuthorityStatus::Insufficient
    } else if reports
        .iter()
        .all(|report| report.status == AuthorityStatus::Pass)
    {
        AuthorityStatus::Pass
    } else {
        AuthorityStatus::Fail
    };
    let runs_recomputed = reports.iter().map(|report| report.runs_recomputed).sum();
    let policy_failure_count = reports
        .iter()
        .map(|report| report.policy_failure_count)
        .sum();
    let diagnostics = reports
        .iter()
        .flat_map(|report| report.diagnostics.iter().cloned())
        .collect();
    (
        SecurityAuthorityVerdict {
            status,
            runs_recomputed,
            policy_failure_count,
            reports,
            diagnostics,
        },
        failures,
    )
}

fn security_report_coverage_diagnostics(
    report: &super::types::VerifiedArtifact<super::types::PublicBenchmarkReport>,
    runs: &[&super::types::VerifiedArtifact<super::types::MemoryRunArtifact>],
    verified: &VerifiedBenchmarkArtifacts,
) -> Vec<String> {
    let suites = verified
        .memory_suites
        .iter()
        .filter(|suite| {
            suite.value.benchmark_id == report.value.benchmark_id
                && suite.value.version == report.value.benchmark_version
                && report.value.suite.as_deref() == Some(suite.value.suite.as_str())
        })
        .collect::<Vec<_>>();
    let Some(suite) = (suites.len() == 1).then(|| suites[0]) else {
        return vec![format!(
            "security report requires exactly one matching typed suite; found {}",
            suites.len()
        )];
    };
    let mut diagnostics = Vec::new();
    if suite.sha256 != REGISTERED_ADVERSARIAL_SUITE_SHA256 {
        diagnostics.push(
            "security report does not use the registered adversarial security suite identity"
                .to_string(),
        );
    }
    let expected_suite_identity = format!("sha256-raw-suite-v1:{}", suite.sha256);
    if report
        .value
        .aggregate_metrics
        .get("suite_content_identity")
        .and_then(Value::as_str)
        != Some(expected_suite_identity.as_str())
    {
        diagnostics
            .push("security report suite identity does not match consumed suite bytes".to_string());
    }
    let expected = suite
        .value
        .tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeMap::new();
    for run in runs {
        *observed.entry(run.value.task_id.as_str()).or_insert(0usize) += 1;
    }
    let missing = expected
        .iter()
        .filter(|task| !observed.contains_key(**task))
        .copied()
        .collect::<Vec<_>>();
    let duplicate = observed
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(task, _)| *task)
        .collect::<Vec<_>>();
    let extra = observed
        .keys()
        .filter(|task| !expected.contains(**task))
        .copied()
        .collect::<Vec<_>>();
    if report.value.conditions.len() != 1 || report.value.conditions[0] != "remem_default" {
        diagnostics.push("security report conditions must be exactly remem_default".to_string());
    }
    if runs.len() != expected.len()
        || !missing.is_empty()
        || !duplicate.is_empty()
        || !extra.is_empty()
    {
        diagnostics.push(format!(
            "security report must cover the exact typed suite task set once: expected={} observed={} missing=[{}] duplicate=[{}] extra=[{}]",
            expected.len(),
            runs.len(),
            missing.join(","),
            duplicate.join(","),
            extra.join(",")
        ));
    }
    let wrong_conditions = runs
        .iter()
        .filter(|run| run.value.condition != "remem_default")
        .map(|run| format!("{}={}", run.value.task_id, run.value.condition))
        .collect::<Vec<_>>();
    if !wrong_conditions.is_empty() {
        diagnostics.push(format!(
            "security report runs must all use remem_default: {}",
            wrong_conditions.join(",")
        ));
    }
    diagnostics
}

struct SecurityReportBinding {
    target: Option<String>,
    models: Vec<Value>,
    model_execution_identity_consistent: bool,
    platforms: Vec<String>,
    producing_shas: Vec<String>,
    production_input_trees: Vec<String>,
    source_dirty_attestations: Vec<Option<bool>>,
    commit_tree_verified: bool,
    ready: bool,
}

fn security_report_binding(
    runs: &[&super::types::VerifiedArtifact<super::types::MemoryRunArtifact>],
) -> SecurityReportBinding {
    let mut models = BTreeMap::new();
    let mut model_execution_identities = BTreeSet::new();
    let mut platforms = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut producing_shas = BTreeSet::new();
    let mut production_input_trees = BTreeSet::new();
    let mut source_dirty_attestations = BTreeSet::new();
    let mut resolved_commit_trees = BTreeMap::new();
    let mut commit_tree_verified = !runs.is_empty();
    let all_runs_bound = !runs.is_empty()
        && runs.iter().all(|run| {
            let environment = &run.value.environment;
            let model_key = serde_json::to_string(&run.value.reader_model).unwrap_or_default();
            models.insert(model_key, run.value.reader_model.clone());
            let mut execution_identity = run.value.reader_model.clone();
            if let Some(object) = execution_identity.as_object_mut() {
                object.remove("prompt_hash");
            }
            model_execution_identities
                .insert(serde_json::to_string(&execution_identity).unwrap_or_default());
            platforms.insert(format!("{}/{}", environment.os, environment.arch));
            if let Some(target) = target_triple(&environment.os, &environment.arch) {
                targets.insert(target);
            }
            producing_shas.insert(environment.remem_commit.clone());
            let Some(tree) = environment.production_input_tree_sha256.as_deref() else {
                return false;
            };
            production_input_trees.insert(tree.to_string());
            source_dirty_attestations.insert(environment.source_dirty);
            let resolved_tree = resolved_commit_trees
                .entry(environment.remem_commit.clone())
                .or_insert_with(|| {
                    implementation::production_input_tree_sha256_for_commit(
                        &environment.remem_commit,
                    )
                });
            let commit_tree_matches = resolved_tree.as_deref() == Some(tree);
            commit_tree_verified &= commit_tree_matches;
            model_identity_is_complete(&run.value.reader_model)
                && is_lower_hex(&environment.remem_commit, 40)
                && is_lower_hex(tree, 64)
                && commit_tree_matches
                && environment.source_dirty == Some(false)
        });
    let ready = all_runs_bound
        && !models.is_empty()
        && model_execution_identities.len() == 1
        && platforms.len() == 1
        && targets.len() == 1
        && producing_shas.len() == 1
        && production_input_trees.len() == 1
        && source_dirty_attestations == BTreeSet::from([Some(false)]);
    SecurityReportBinding {
        target: (targets.len() == 1)
            .then(|| targets.into_iter().next())
            .flatten(),
        models: models.into_values().collect(),
        model_execution_identity_consistent: model_execution_identities.len() == 1,
        platforms: platforms.into_iter().collect(),
        producing_shas: producing_shas.into_iter().collect(),
        production_input_trees: production_input_trees.into_iter().collect(),
        source_dirty_attestations: source_dirty_attestations.into_iter().collect(),
        commit_tree_verified,
        ready,
    }
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn model_identity_is_complete(model: &Value) -> bool {
    let Some(object) = model.as_object() else {
        return false;
    };
    let provider = object
        .get("provider")
        .or_else(|| object.get("agent"))
        .and_then(Value::as_str);
    let name = object.get("model").and_then(Value::as_str);
    provider.is_some_and(|value| !value.trim().is_empty())
        && name.is_some_and(|value| !value.trim().is_empty())
}

fn evaluate_release(
    verified: &VerifiedBenchmarkArtifacts,
    security: &SecurityAuthorityVerdict,
    implementation: &super::types::ImplementationAuthorityBinding,
) -> ReleaseAuthorityVerdict {
    let current_tree = implementation.build_production_input_tree_sha256.as_deref();
    let implementation_current = implementation_allows_release(implementation);
    let mut current = BTreeSet::new();
    let mut stale = BTreeSet::new();
    for report in &security.reports {
        let Some(target) = report.target.as_deref() else {
            continue;
        };
        let run_paths = verified
            .reports
            .iter()
            .find(|candidate| candidate.path == report.report_path)
            .map(|candidate| &candidate.value.run_artifacts);
        let source_current = implementation_current
            && report.status == AuthorityStatus::Pass
            && current_tree.is_some()
            && run_paths.is_some_and(|paths| {
                paths.iter().all(|path| {
                    verified
                        .memory_runs
                        .iter()
                        .find(|run| run.path == *path)
                        .is_some_and(|run| {
                            run.value.environment.source_dirty == Some(false)
                                && run
                                    .value
                                    .environment
                                    .production_input_tree_sha256
                                    .as_deref()
                                    == current_tree
                        })
                })
            });
        if source_current {
            current.insert(target.to_string());
        } else {
            stale.insert(target.to_string());
        }
    }
    let required = REQUIRED_RELEASE_TARGETS
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let missing = required
        .iter()
        .filter(|target| !current.contains(target.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let ready = release_is_ready(&missing, &stale);
    let mut diagnostics = Vec::new();
    if !implementation_current {
        diagnostics.push(
            "runtime implementation identity is unavailable, dirty, or source-inequivalent"
                .to_string(),
        );
    }
    if !ready {
        diagnostics.push(format!(
            "missing genuine current evidence for targets: {}",
            missing.join(", ")
        ));
    }
    if !stale.is_empty() {
        diagnostics.push(format!(
            "stale evidence remains for targets: {}",
            stale.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    ReleaseAuthorityVerdict {
        status: if ready {
            AuthorityStatus::Pass
        } else {
            AuthorityStatus::Insufficient
        },
        ready,
        required_targets: required,
        current_targets: current.into_iter().collect(),
        missing_targets: missing,
        stale_targets: stale.into_iter().collect(),
        diagnostics,
    }
}

fn release_is_ready(missing: &[String], stale: &BTreeSet<String>) -> bool {
    missing.is_empty() && stale.is_empty()
}

pub(in crate::eval::bench_artifact) fn implementation_allows_release(
    implementation: &super::types::ImplementationAuthorityBinding,
) -> bool {
    implementation.executable_source_equivalent
        && implementation.build_source_dirty == Some(false)
        && implementation.checkout_source_dirty == Some(false)
}

fn target_triple(os: &str, arch: &str) -> Option<String> {
    match (os, arch) {
        ("macos", "x86_64") => Some("x86_64-apple-darwin".to_string()),
        ("macos", "aarch64") => Some("aarch64-apple-darwin".to_string()),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu".to_string()),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod registered_security_suite_tests {
    use std::collections::BTreeSet;

    use sha2::{Digest, Sha256};

    use super::{release_is_ready, REGISTERED_ADVERSARIAL_SUITE_SHA256};

    #[test]
    fn registered_suite_digest_matches_checked_in_bytes() {
        let bytes =
            include_bytes!("../../../eval/public/memory/suites/adversarial-policy/suite.json");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            REGISTERED_ADVERSARIAL_SUITE_SHA256
        );
    }

    #[test]
    fn release_rejects_stale_duplicate_of_current_target() {
        let missing = Vec::new();
        let stale = BTreeSet::from(["aarch64-apple-darwin".to_string()]);

        assert!(!release_is_ready(&missing, &stale));
    }
}
