use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use super::fixture::load_suite;
use super::runner::{run_memory_bench, MemoryBenchOptions};
use super::types::{MemoryBenchCondition, DEFAULT_PUBLIC_ROOT, DEFAULT_SUITE};

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
fn remem_default_memory_bench_writes_verifiable_public_artifacts() -> Result<()> {
    let root = unique_temp_dir("remem-memory-bench-public")?;
    copy_dir_all(std::path::Path::new(DEFAULT_PUBLIC_ROOT), &root)?;
    let report_path = root.join("memory/reports/remem-code-memory-v1.json");
    let report = run_memory_bench(MemoryBenchOptions {
        suite: DEFAULT_SUITE.to_string(),
        condition: Some("remem_default".to_string()),
        json_out: report_path.to_string_lossy().to_string(),
        root: root.to_string_lossy().to_string(),
        artifact_prefix: Some("memory/artifacts/remem-code-memory-v1".to_string()),
    })?;

    assert_eq!(report.conditions, vec!["remem_default"]);
    assert_eq!(report.run_artifacts.len(), 8);
    let metrics = &report.aggregate_metrics;
    assert_eq!(metrics["run_count"], 8);
    assert_eq!(metrics["overall"]["tasks"], 8);
    assert_eq!(metrics["overall"]["support_coverage"], 1.0);
    assert!(metrics["by_category"]["prior_bug_root_cause"].is_object());

    let verify = crate::eval::bench_artifact::verify_benchmark_artifacts(
        crate::eval::bench_artifact::BenchVerifyOptions { root },
    )?;
    assert!(verify.passed, "{:#?}", verify.failures);
    assert!(verify.run_artifacts_checked >= 10);
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
