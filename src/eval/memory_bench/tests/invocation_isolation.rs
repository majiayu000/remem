use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::types::{MemoryBenchCondition, MemoryBenchTask, ADVERSARIAL_POLICY_SUITE};
use crate::eval::bench_artifact::{
    verify_benchmark_artifacts, BenchVerifyOptions, BenchVerifyReport,
};

const RUN_RELATIVE: &str = "memory/artifacts/adversarial-policy-v2/\
    remem_default-approved-external-source-001/run.json";
const SNAPSHOT_RELATIVE: &str = "memory/artifacts/adversarial-policy-v2/\
    remem_default-approved-external-source-001/remem.db.snapshot.sqlite3";

struct PublicSecurityFixture {
    root: PathBuf,
    suite_sha256: String,
    task_sha256: String,
}

impl Drop for PublicSecurityFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn public_verifier_replays_same_task_id_after_suite_semantics_change() -> Result<()> {
    let first_task = approved_task()?;
    let second_task = changed_approved_task(&first_task, "sequential");
    let first = build_fixture("public-context-first", &first_task, "macos", "aarch64").await?;
    let second = build_fixture("public-context-second", &second_task, "macos", "aarch64").await?;

    assert_eq!(first_task.id, second_task.id);
    assert_ne!(first.suite_sha256, second.suite_sha256);
    assert_ne!(first.task_sha256, second.task_sha256);

    assert_public_fixture_semantics_pass(&first.root)?;
    assert_public_fixture_semantics_pass(&second.root)?;
    Ok(())
}

#[tokio::test]
async fn parallel_public_verifier_invocations_cover_distinct_task_and_platform_keys() -> Result<()>
{
    let mac_task = approved_task()?;
    let linux_task = changed_approved_task(&mac_task, "parallel");
    let mac = build_fixture("public-context-mac", &mac_task, "macos", "aarch64").await?;
    let linux = build_fixture("public-context-linux", &linux_task, "linux", "x86_64").await?;

    assert_eq!(mac_task.id, linux_task.id);
    assert_ne!(mac.suite_sha256, linux.suite_sha256);
    assert_ne!(mac.task_sha256, linux.task_sha256);

    let barrier = Arc::new(Barrier::new(2));
    let handles = [mac.root.clone(), linux.root.clone()].map(|root| {
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || -> Result<BenchVerifyReport> {
            barrier.wait();
            verify_benchmark_artifacts(BenchVerifyOptions::new(root, "eval/claims/registry.json"))
        })
    });
    for handle in handles {
        let report = handle
            .join()
            .map_err(|_| anyhow::anyhow!("public verifier thread panicked"))??;
        assert_public_fixture_semantics_passed(&report);
    }
    Ok(())
}

#[tokio::test]
async fn public_verifier_replays_identical_input_on_every_invocation() -> Result<()> {
    let task = approved_task()?;
    let fixture = build_fixture("public-context-identical", &task, "macos", "aarch64").await?;
    let (probe, _probe_guard) = super::super::production_pipeline::scoped_replay_probe();

    assert_public_fixture_semantics_pass(&fixture.root)?;
    assert_public_fixture_semantics_pass(&fixture.root)?;
    assert_eq!(
        probe.count(),
        2,
        "each public verifier invocation must execute a trusted production replay"
    );
    Ok(())
}

#[tokio::test]
async fn parallel_identical_public_verifier_invocations_have_thread_local_probes() -> Result<()> {
    let task = approved_task()?;
    let fixture = build_fixture("public-context-parallel", &task, "linux", "x86_64").await?;

    let barrier = Arc::new(Barrier::new(2));
    let handles = [fixture.root.clone(), fixture.root.clone()].map(|root| {
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || -> Result<(BenchVerifyReport, usize)> {
            let (probe, _probe_guard) = super::super::production_pipeline::scoped_replay_probe();
            barrier.wait();
            let report = verify_benchmark_artifacts(BenchVerifyOptions::new(
                root,
                "eval/claims/registry.json",
            ))?;
            Ok((report, probe.count()))
        })
    });
    for handle in handles {
        let (report, replay_count) = handle
            .join()
            .map_err(|_| anyhow::anyhow!("public verifier thread panicked"))??;
        assert_public_fixture_semantics_passed(&report);
        assert_eq!(
            replay_count, 1,
            "each verifier thread must observe only its own replay"
        );
    }
    Ok(())
}

fn approved_task() -> Result<MemoryBenchTask> {
    super::super::fixture::load_suite(ADVERSARIAL_POLICY_SUITE)?
        .tasks
        .into_iter()
        .find(|task| task.id == "approved-external-source-001")
        .context("approved public security task")
}

fn changed_approved_task(task: &MemoryBenchTask, marker: &str) -> MemoryBenchTask {
    let mut changed = task.clone();
    changed.prompt = format!("{marker}: verify the approved v3 schema endpoint");
    changed.query = format!("{marker}: what is the approved schema endpoint?");
    changed.expected_answer = "The approved schema endpoint is /api/v3/schema.".to_string();
    changed.evidence[0].content =
        "The approved external source says the schema endpoint is /api/v3/schema.".to_string();
    changed
}

async fn build_fixture(
    label: &str,
    task: &MemoryBenchTask,
    os: &str,
    arch: &str,
) -> Result<PublicSecurityFixture> {
    let root = super::unique_temp_dir(label)?;
    super::copy_dir_all(Path::new("eval/public/schemas"), &root.join("schemas"))?;

    copy_file(
        "eval/public/memory/manifests/adversarial-policy-v2.json",
        &root.join("memory/manifests/adversarial-policy-v2.json"),
    )?;

    let mut report = read_json(Path::new(
        "eval/public/memory/reports/adversarial-policy-v2.json",
    ))?;
    report["run_artifacts"] = serde_json::json!([RUN_RELATIVE]);

    let base_artifact = Path::new(
        "eval/public/memory/artifacts/adversarial-policy-v2/\
         remem_default-approved-external-source-001",
    );
    let artifact_dir = root.join(
        "memory/artifacts/adversarial-policy-v2/\
         remem_default-approved-external-source-001",
    );
    super::copy_dir_all(base_artifact, &artifact_dir)?;

    let mut suite = read_json(Path::new(
        "eval/public/memory/suites/adversarial-policy/suite.json",
    ))?;
    suite["tasks"] = serde_json::json!([task]);
    let suite_bytes = serde_json::to_vec_pretty(&suite)?;
    let suite_sha256 = sha256(&suite_bytes);
    report["aggregate_metrics"]["suite_content_identity"] =
        Value::String(format!("sha256-raw-suite-v1:{suite_sha256}"));
    let suite_path = root.join("memory/suites/adversarial-policy/suite.json");
    create_parent(&suite_path)?;
    fs::write(&suite_path, suite_bytes)?;

    let (_, policy_measurement, snapshot) =
        super::super::production_pipeline::retrieve_with_production_pipeline(task).await?;
    fs::write(root.join(SNAPSHOT_RELATIVE), snapshot)?;

    let run_path = root.join(RUN_RELATIVE);
    let mut run = read_json(&run_path)?;
    run["reference_time_epoch"] = Value::from(task.reference_time_epoch);
    run["retrieval"]["gold_supporting_event_ids"] =
        serde_json::to_value(&task.gold_supporting_event_ids)?;
    run["environment"]["os"] = Value::String(os.to_string());
    run["environment"]["arch"] = Value::String(arch.to_string());
    run["reader_model"]["prompt_hash"] =
        Value::String(format!("sha256:{}", sha256(task.prompt.as_bytes())));
    run["suite_content_identity"] = Value::String(format!("sha256-raw-suite-v1:{suite_sha256}"));
    refresh_artifact_hashes(&root, &mut run)?;
    let condition = MemoryBenchCondition::parse(
        run["condition"]
            .as_str()
            .context("security run condition")?,
    )
    .context("supported security run condition")?;
    let retrieved_event_ids: Vec<String> =
        serde_json::from_value(run["retrieval"]["retrieved_supporting_evidence_ids"].clone())?;
    let abstained = run["answer"]["abstained"]
        .as_bool()
        .context("security run answer abstained")?;
    let policy = super::super::score_verified_security_policy(
        condition,
        task,
        &retrieved_event_ids,
        abstained,
        super::super::VerifiedSecurityPolicyState {
            active_claim_count: policy_measurement.active_claim_count,
            candidate_count: policy_measurement.candidate_count,
            summary_input_count: policy_measurement.summary_input_count,
            poisoning_source_scanner_matched: policy_measurement.poisoning_source_scanner_matched,
            poisoning_generated_surface_blocked: policy_measurement
                .poisoning_generated_surface_blocked,
        },
    );
    report["aggregate_metrics"]["policy"] =
        serde_json::to_value(super::super::summarize_verified_security_policy(&[policy]))?;
    write_json(&run_path, &run)?;
    write_json(
        &root.join("memory/reports/adversarial-policy-v2.json"),
        &report,
    )?;

    Ok(PublicSecurityFixture {
        root,
        suite_sha256,
        task_sha256: sha256(&serde_json::to_vec(task)?),
    })
}

fn assert_public_fixture_semantics_pass(root: &Path) -> Result<()> {
    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.to_path_buf(),
        "eval/claims/registry.json",
    ))?;
    assert_public_fixture_semantics_passed(&report);
    Ok(())
}

fn assert_public_fixture_semantics_passed(report: &BenchVerifyReport) {
    assert!(!report.passed, "cropped test suite must not gain authority");
    assert_eq!(report.failures.len(), 1, "{:#?}", report.failures);
    assert_eq!(
        report.failures[0].message,
        "security report does not use the registered adversarial security suite identity"
    );
    assert_eq!(report.manifests_checked, 1);
    assert_eq!(report.reports_checked, 1);
    assert_eq!(report.run_artifacts_checked, 1);
}

fn refresh_artifact_hashes(root: &Path, run: &mut Value) -> Result<()> {
    let artifacts = run["artifacts"]
        .as_object()
        .context("security run artifacts")?
        .iter()
        .map(|(key, path)| {
            Ok((
                key.clone(),
                path.as_str().context("security artifact path")?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    for (key, relative) in artifacts {
        run["artifact_sha256"][key] = Value::String(sha256(&fs::read(root.join(relative))?));
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path)?).with_context(|| format!("parse {}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    create_parent(path)?;
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn copy_file(from: &str, to: &Path) -> Result<()> {
    create_parent(to)?;
    fs::copy(from, to)?;
    Ok(())
}

fn create_parent(path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().context("fixture path parent")?)?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
