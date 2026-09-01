use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use super::fixture::{
    load_suite, load_suite_file_with_content_identity, validate_suite, validate_suite_selection,
};
use super::runner::{is_checked_in_public_root, run_memory_bench, MemoryBenchOptions};
use super::types::{
    MemoryBenchCondition, ADVERSARIAL_POLICY_SUITE, DEFAULT_PUBLIC_ROOT, DEFAULT_SUITE,
};

mod invocation_isolation;

#[test]
fn remem_code_memory_fixture_covers_required_categories() -> Result<()> {
    let fixture = load_suite(DEFAULT_SUITE)?;
    let categories = fixture
        .tasks
        .iter()
        .map(|task| task.category.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for required in [
        "temporal_as_of",
        "stale_memory_avoidance",
        "conflict_detection",
        "workstream_continuity",
        "prior_bug_root_cause",
        "architecture_constraints",
        "file_source_anchors",
        "user_context_relevance",
    ] {
        assert!(
            categories.contains(required),
            "missing required memory bench category {required}"
        );
    }
    assert!(fixture.tasks.iter().all(|task| {
        !task.gold_supporting_event_ids.is_empty()
            && task.gold_supporting_event_ids.iter().all(|id| {
                task.evidence
                    .iter()
                    .any(|evidence| evidence.event_id == *id)
            })
    }));
    Ok(())
}

#[test]
fn memory_bench_conditions_are_supported() {
    for condition in MemoryBenchCondition::ALL {
        assert_eq!(
            MemoryBenchCondition::parse(condition.as_str()),
            Some(condition)
        );
    }
    assert_eq!(MemoryBenchCondition::parse("unknown"), None);
}

#[test]
fn two_missing_roots_are_not_treated_as_the_same_public_root() {
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    );
    let requested = std::env::temp_dir().join(format!("remem-missing-requested-{suffix}"));
    assert!(!requested.exists());
    assert!(!is_checked_in_public_root(&requested));
}

#[test]
fn memory_bench_fixture_allows_suite_to_differ_from_benchmark_id() -> Result<()> {
    let mut fixture = load_suite(DEFAULT_SUITE)?;
    fixture.benchmark_id = "independent-benchmark-id".to_string();

    validate_suite(&fixture)?;
    validate_suite_selection(&fixture, DEFAULT_SUITE)?;
    assert_ne!(fixture.suite, fixture.benchmark_id);
    Ok(())
}

#[test]
fn memory_bench_fixture_rejects_mismatched_requested_suite() -> Result<()> {
    let fixture = load_suite(DEFAULT_SUITE)?;

    assert!(validate_suite_selection(&fixture, "misrouted-suite")
        .unwrap_err()
        .to_string()
        .contains("must match requested suite"));
    Ok(())
}

#[tokio::test]
async fn public_artifact_prefix_rejects_parent_traversal_before_writing() -> Result<()> {
    let root = unique_temp_dir("remem-memory-bench-prefix-traversal")?;
    copy_dir_all(std::path::Path::new(DEFAULT_PUBLIC_ROOT), &root)?;
    let escaped_name = format!(
        "escaped-{}",
        root.file_name()
            .expect("temporary root name")
            .to_string_lossy()
    );
    let escaped = root
        .parent()
        .expect("temporary root parent")
        .join(&escaped_name);
    let result = run_memory_bench(MemoryBenchOptions {
        suite: DEFAULT_SUITE.to_string(),
        condition: Some("no_memory".to_string()),
        json_out: root
            .join("memory/reports/prefix-traversal.json")
            .to_string_lossy()
            .to_string(),
        root: root.to_string_lossy().to_string(),
        artifact_prefix: Some(format!("../{escaped_name}")),
    })
    .await;

    assert!(result
        .expect_err("parent traversal must fail")
        .to_string()
        .contains("artifact prefix"));
    assert!(!escaped.exists());
    Ok(())
}

#[test]
fn adversarial_policy_fixture_covers_required_categories() -> Result<()> {
    let fixture = load_suite(ADVERSARIAL_POLICY_SUITE)?;
    let categories = fixture
        .tasks
        .iter()
        .map(|task| task.category.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for required in [
        "secrets_api_keys",
        "credentials",
        "payments_accounts",
        "unframed_third_party_personal_data",
        "jokes_roleplay",
        "negation",
        "unsupported_assistant_claims",
        "unapproved_external_source_claims",
        "cross_sentence_splicing",
        "same_name_repos",
        "multi_task_sessions",
        "branch_divergence",
        "stale_file_anchors",
        "conflicting_memories",
        "instruction_injection",
        "authority_claim",
        "opaque_payload",
        "benign_quoted_instruction",
    ] {
        assert!(
            categories.contains(required),
            "missing required adversarial policy category {required}"
        );
    }
    assert!(fixture.tasks.iter().any(|task| {
        task.category == "approved_external_source_claims"
            && task
                .policy
                .as_ref()
                .is_some_and(|policy| policy.explicit_approval)
    }));
    assert!(fixture.tasks.iter().all(|task| {
        task.policy.as_ref().is_some_and(|policy| {
            policy.explicit_approval
                || policy.poisoning_quarantine_expected
                || (policy.non_retention_required
                    && policy.expected_active_claims == 0
                    && policy.expected_candidates == 0
                    && policy.expected_summary_inputs == 0
                    && policy.expected_policy_abstention)
        })
    }));
    // GH-855: every poisoning fixture must be caught by the production
    // scanner, and quarantine must win over retention_allowed.
    for task in fixture.tasks.iter().filter(|task| {
        task.policy
            .as_ref()
            .is_some_and(|policy| policy.poisoning_quarantine_expected)
    }) {
        assert!(
            task.evidence.iter().any(|evidence| {
                crate::memory::poisoning::scan_instruction_pattern(&evidence.content).is_some()
            }),
            "poisoning fixture {} must match the production pattern scanner",
            task.id
        );
        let outcome = super::diagnostics::score_policy(
            MemoryBenchCondition::RememDefault,
            task,
            &[],
            task.abstention_allowed,
            None,
        );
        assert_eq!(
            outcome.active_claim_count, 0,
            "poisoning fixture {} must never produce an active claim",
            task.id
        );
        assert!(outcome.poisoning_scanner_matched);
        assert_eq!(
            outcome.policy_failure_count, 0,
            "poisoning fixture {} must pass policy scoring",
            task.id
        );
    }
    Ok(())
}

#[test]
fn runtime_suite_identity_changes_when_same_task_prompt_and_expected_change() -> Result<()> {
    let temp = unique_temp_dir("remem-runtime-suite-identity")?;
    fs::create_dir_all(&temp)?;
    let path = temp.join("suite.json");
    let mut suite: serde_json::Value = serde_json::from_slice(&fs::read(
        super::fixture::suite_path(ADVERSARIAL_POLICY_SUITE),
    )?)?;
    fs::write(&path, serde_json::to_vec_pretty(&suite)?)?;
    let (original, original_identity) =
        load_suite_file_with_content_identity(&path, ADVERSARIAL_POLICY_SUITE)?;

    suite["tasks"][0]["prompt"] = serde_json::json!("runtime-mutated prompt");
    suite["tasks"][0]["expected_answer"] = serde_json::json!("runtime-mutated expected");
    fs::write(&path, serde_json::to_vec_pretty(&suite)?)?;
    let (mutated, mutated_identity) =
        load_suite_file_with_content_identity(&path, ADVERSARIAL_POLICY_SUITE)?;

    assert_eq!(original.tasks[0].id, mutated.tasks[0].id);
    assert_ne!(original_identity, mutated_identity);
    assert!(mutated_identity.starts_with("sha256-raw-suite-v1:"));
    fs::remove_dir_all(temp)?;
    Ok(())
}

#[tokio::test]
async fn remem_default_memory_bench_writes_verifiable_public_artifacts() -> Result<()> {
    let root = unique_temp_dir("remem-memory-bench-public")?;
    copy_dir_all(std::path::Path::new(DEFAULT_PUBLIC_ROOT), &root)?;
    let report_path = root.join("memory/reports/remem-code-memory-v1.json");
    let report = run_memory_bench(MemoryBenchOptions {
        suite: DEFAULT_SUITE.to_string(),
        condition: Some("remem_default".to_string()),
        json_out: report_path.to_string_lossy().to_string(),
        root: root.to_string_lossy().to_string(),
        artifact_prefix: Some("memory/artifacts/remem-code-memory-v1".to_string()),
    })
    .await?;

    assert_eq!(report.conditions, vec!["remem_default"]);
    assert_eq!(report.run_artifacts.len(), 8);
    let metrics = &report.aggregate_metrics;
    assert_eq!(metrics["run_count"], 8);
    assert_eq!(metrics["overall"]["tasks"], 8);
    assert_eq!(metrics["overall"]["support_coverage"], 1.0);
    assert!(metrics["by_category"]["prior_bug_root_cause"].is_object());

    let verify = crate::eval::bench_artifact::verify_benchmark_artifacts(
        crate::eval::bench_artifact::BenchVerifyOptions::new(root, "eval/claims/registry.json"),
    )?;
    assert!(verify.passed, "{:#?}", verify.failures);
    assert!(verify.run_artifacts_checked >= 10);
    Ok(())
}

#[tokio::test]
async fn adversarial_policy_bench_reports_zero_policy_leaks() -> Result<()> {
    let root = unique_temp_dir("remem-adversarial-policy-public")?;
    copy_dir_all(std::path::Path::new(DEFAULT_PUBLIC_ROOT), &root)?;
    let report_path = root.join("memory/reports/adversarial-policy-v2.json");
    let report = run_memory_bench(MemoryBenchOptions {
        suite: ADVERSARIAL_POLICY_SUITE.to_string(),
        condition: Some("remem_default".to_string()),
        json_out: report_path.to_string_lossy().to_string(),
        root: root.to_string_lossy().to_string(),
        artifact_prefix: Some("memory/artifacts/adversarial-policy-v2".to_string()),
    })
    .await?;

    assert_eq!(report.conditions, vec!["remem_default"]);
    assert_eq!(report.benchmark_version, "v2");
    assert_eq!(report.run_artifacts.len(), 20);
    let suite_content_identity = report.aggregate_metrics["suite_content_identity"]
        .as_str()
        .expect("report suite content identity");
    for run_path in &report.run_artifacts {
        let run: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join(run_path))?)?;
        assert_eq!(
            run["benchmark_version"], report.benchmark_version,
            "generated run {run_path} must inherit the suite version"
        );
        assert_eq!(
            run["suite_content_identity"], suite_content_identity,
            "generated run {run_path} must bind the exact suite bytes consumed by the runner"
        );
        assert!(run["environment"]["source_dirty"].is_boolean());
        assert_eq!(
            run["environment"]["production_input_tree_sha256"]
                .as_str()
                .map(str::len),
            Some(64)
        );
    }
    let policy = &report.aggregate_metrics["policy"];
    assert_eq!(policy["non_retention_leak_rate"], 0.0);
    assert_eq!(policy["false_block_rate"], 0.0);
    assert_eq!(policy["suppression_obeyed_rate"], 1.0);
    assert_eq!(policy["sensitive_restricted_default_exclusion_rate"], 1.0);
    assert_eq!(policy["policy_abstention_accuracy"], 1.0);
    assert_eq!(policy["policy_failure_rate"], 0.0);
    assert_eq!(
        report.aggregate_metrics["verification_paths"],
        serde_json::json!(["capture_observation_candidate_promotion"])
    );
    assert_eq!(
        report.aggregate_metrics["measurement_sources"],
        serde_json::json!(["sqlite_production_tables"])
    );

    let approved_path = report
        .run_artifacts
        .iter()
        .find(|path| path.contains("remem_default-approved-external-source-001/"))
        .expect("approved production-path artifact");
    let approved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join(approved_path))?)?;
    assert_eq!(approved["metrics"]["policy"]["active_claim_count"], 1);
    assert_eq!(approved["metrics"]["policy"]["candidate_count"], 0);
    assert_eq!(approved["metrics"]["policy"]["summary_input_count"], 1);
    assert_eq!(
        approved["metrics"]["policy"]["verification_path"],
        "capture_observation_candidate_promotion"
    );

    let opaque_path = report
        .run_artifacts
        .iter()
        .find(|path| path.contains("opaque-payload-001"))
        .expect("opaque production-path artifact");
    let opaque: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join(opaque_path))?)?;
    assert_eq!(
        opaque["metrics"]["policy"]["poisoning_source_scanner_matched"],
        false
    );
    assert_eq!(
        opaque["metrics"]["policy"]["poisoning_generated_surface_blocked"],
        true
    );
    assert_eq!(opaque["metrics"]["policy"]["policy_failure_count"], 0);

    let verify = crate::eval::bench_artifact::verify_benchmark_artifacts(
        crate::eval::bench_artifact::BenchVerifyOptions::new(root, "eval/claims/registry.json"),
    )?;
    assert!(verify.passed, "{:#?}", verify.failures);
    assert!(verify.run_artifacts_checked >= 25);
    Ok(())
}

#[tokio::test]
async fn write_vs_retrieval_report_includes_diagnostics_baselines_and_performance() -> Result<()> {
    let root = unique_temp_dir("remem-write-vs-retrieval-public")?;
    copy_dir_all(std::path::Path::new(DEFAULT_PUBLIC_ROOT), &root)?;
    let report_path = root.join("memory/reports/write-vs-retrieval.json");
    let report = run_memory_bench(MemoryBenchOptions {
        suite: DEFAULT_SUITE.to_string(),
        condition: None,
        json_out: report_path.to_string_lossy().to_string(),
        root: root.to_string_lossy().to_string(),
        artifact_prefix: Some("memory/artifacts/write-vs-retrieval".to_string()),
    })
    .await?;

    for condition in [
        "truncated_full_context",
        "oracle_evidence",
        "complete_stored_memory",
        "retrieved_memory",
        "bm25_baseline",
        "vector_baseline",
        "hybrid_rag_baseline",
        "summary_baseline",
    ] {
        assert!(
            report.conditions.iter().any(|item| item == condition),
            "missing condition {condition}"
        );
        assert!(
            report.aggregate_metrics["failure_decomposition"]["by_condition"][condition]
                .is_object(),
            "missing failure decomposition for {condition}"
        );
        assert!(
            report.aggregate_metrics["performance"][condition].is_object(),
            "missing performance metrics for {condition}"
        );
    }
    assert_eq!(report.aggregate_metrics["run_count"], 80);
    assert!(
        report.aggregate_metrics["failure_decomposition"]["overall"]["retrieval_miss"]
            .as_u64()
            .is_some()
    );
    assert!(report.aggregate_metrics["performance"]["retrieved_memory"]
        ["retrieval_latency_p95_ms"]
        .as_f64()
        .is_some());
    Ok(())
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf> {
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&root)?;
    Ok(root)
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
