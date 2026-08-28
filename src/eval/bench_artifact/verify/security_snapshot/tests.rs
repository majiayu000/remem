use std::collections::BTreeMap;
use std::sync::{Arc, Barrier, Mutex};

use anyhow::Result;

use super::{TrustedSnapshotCacheKey, VerificationContext};
use crate::eval::memory_bench::types::{MemoryBenchSuiteFixture, MemoryBenchTask};
use crate::eval::security_snapshot_identity::SnapshotIdentity;

fn task() -> Result<MemoryBenchTask> {
    let suite: MemoryBenchSuiteFixture = serde_json::from_str(include_str!(
        "../../../../../eval/public/memory/suites/adversarial-policy/suite.json"
    ))?;
    Ok(suite
        .tasks
        .into_iter()
        .next()
        .expect("public suite has tasks"))
}

fn identity(marker: &str) -> SnapshotIdentity {
    BTreeMap::from([("sqlite_schema".to_string(), marker.to_string())])
}

#[test]
fn cache_key_binds_suite_task_pathspec_and_platform() -> Result<()> {
    let task = task()?;
    let mac = TrustedSnapshotCacheKey::new("suite-a", &task, "macos", "aarch64")?;
    let other_suite = TrustedSnapshotCacheKey::new("suite-b", &task, "macos", "aarch64")?;
    let linux = TrustedSnapshotCacheKey::new("suite-a", &task, "linux", "x86_64")?;

    assert_ne!(
        mac, other_suite,
        "suite identity must partition replay trust"
    );
    assert_ne!(mac, linux, "artifact platform must partition replay trust");
    assert_eq!(
        mac.production_input_pathspec_sha256,
        super::production_input_pathspec_sha256()
    );
    assert!(!mac.task_semantic_sha256.is_empty());
    Ok(())
}

#[test]
fn same_task_id_in_different_suites_does_not_reuse_replay() -> Result<()> {
    let task = task()?;
    let mut context = VerificationContext::new();
    let mut calls = 0;

    let first = context.trusted_snapshot_identity("suite-a", &task, "macos", "aarch64", |_| {
        calls += 1;
        Ok(identity("suite-a"))
    })?;
    let second = context.trusted_snapshot_identity("suite-b", &task, "macos", "aarch64", |_| {
        calls += 1;
        Ok(identity("suite-b"))
    })?;

    assert_eq!(calls, 2);
    assert_ne!(first, second);
    Ok(())
}

#[test]
fn macos_and_linux_runs_do_not_share_replay_identity() -> Result<()> {
    let task = task()?;
    let mut context = VerificationContext::new();
    let mut calls = 0;

    let mac = context.trusted_snapshot_identity("suite-a", &task, "macos", "aarch64", |_| {
        calls += 1;
        Ok(identity("mac"))
    })?;
    let linux = context.trusted_snapshot_identity("suite-a", &task, "linux", "x86_64", |_| {
        calls += 1;
        Ok(identity("linux"))
    })?;

    assert_eq!(calls, 2);
    assert_ne!(mac, linux);
    Ok(())
}

#[test]
fn consecutive_verification_invocations_replay_after_input_change() -> Result<()> {
    let task = task()?;
    let mut first_invocation = VerificationContext::new();
    let first =
        first_invocation.trusted_snapshot_identity("suite-a", &task, "macos", "aarch64", |_| {
            Ok(identity("before-input-change"))
        })?;

    let mut second_invocation = VerificationContext::new();
    let second = second_invocation.trusted_snapshot_identity(
        "suite-a",
        &task,
        "macos",
        "aarch64",
        |_| Ok(identity("after-input-change")),
    )?;

    assert_ne!(first, second, "a new invocation must not observe old state");
    Ok(())
}

#[test]
fn parallel_verification_invocations_are_isolated() -> Result<()> {
    let task = task()?;
    let barrier = Arc::new(Barrier::new(2));
    let results = Arc::new(Mutex::new(Vec::new()));
    let handles = ["left", "right"].map(|marker| {
        let task = task.clone();
        let barrier = Arc::clone(&barrier);
        let results = Arc::clone(&results);
        std::thread::spawn(move || -> Result<()> {
            let mut context = VerificationContext::new();
            barrier.wait();
            let result =
                context.trusted_snapshot_identity("suite-a", &task, "linux", "x86_64", |_| {
                    Ok(identity(marker))
                })?;
            results.lock().expect("results lock").push(result);
            Ok(())
        })
    });

    for handle in handles {
        handle.join().expect("verification thread")?;
    }
    let mut results = results.lock().expect("results lock").clone();
    results.sort_by(|left, right| left["sqlite_schema"].cmp(&right["sqlite_schema"]));
    assert_eq!(results, vec![identity("left"), identity("right")]);
    Ok(())
}

#[test]
fn failed_replay_is_not_cached() -> Result<()> {
    let task = task()?;
    let mut context = VerificationContext::new();
    let failed = context.trusted_snapshot_identity("suite-a", &task, "linux", "x86_64", |_| {
        anyhow::bail!("replay failed")
    });
    assert!(failed.is_err());

    let retried = context.trusted_snapshot_identity("suite-a", &task, "linux", "x86_64", |_| {
        Ok(identity("retry"))
    })?;
    assert_eq!(retried, identity("retry"));
    Ok(())
}
