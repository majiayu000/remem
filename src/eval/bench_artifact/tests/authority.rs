use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

#[test]
fn verifier_emits_runtime_authority_verdict_with_consumed_byte_hashes() -> Result<()> {
    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(PathBuf::from("eval/public"), "eval/claims/registry.json"))?;

    let serialized = serde_json::to_value(&report)?;
    let verdict = serialized
        .get("authority_verdict")
        .expect("verifier must emit one authority verdict");
    assert_eq!(verdict["schema_version"], 1);
    assert!(verdict["status"].is_string());
    assert!(verdict["consumed_bytes"]
        .as_object()
        .is_some_and(|sources| !sources.is_empty()));
    Ok(())
}

#[test]
fn authority_verdict_has_closed_four_target_release_set() -> Result<()> {
    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(PathBuf::from("eval/public"), "eval/claims/registry.json"))?;
    let verdict = serde_json::to_value(&report)?;
    assert_eq!(
        verdict["authority_verdict"]["release"]["required_targets"],
        serde_json::json!([
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-gnu"
        ])
    );
    assert_eq!(verdict["authority_verdict"]["release"]["ready"], false);
    Ok(())
}

#[test]
fn source_equivalent_build_and_checkout_commits_can_authorize_release() {
    let binding = crate::eval::bench_artifact::authority::implementation::binding_from_parts(
        Some("0123456789abcdef0123456789abcdef01234567"),
        Some("1123456789abcdef0123456789abcdef01234567"),
        Some(false),
        Some(false),
        Some("a".repeat(64).as_str()),
        Some("a".repeat(64).as_str()),
        Some("b".repeat(64).as_str()),
    );

    assert!(binding.executable_source_equivalent);
    assert!(crate::eval::bench_artifact::authority::implementation_allows_release(&binding));
}

#[test]
fn mismatched_production_trees_cannot_authorize_release() {
    let binding = crate::eval::bench_artifact::authority::implementation::binding_from_parts(
        Some("0123456789abcdef0123456789abcdef01234567"),
        Some("0123456789abcdef0123456789abcdef01234567"),
        Some(false),
        Some(false),
        Some("a".repeat(64).as_str()),
        Some("c".repeat(64).as_str()),
        Some("b".repeat(64).as_str()),
    );

    assert!(!binding.executable_source_equivalent);
    assert!(!crate::eval::bench_artifact::authority::implementation_allows_release(&binding));
}

#[test]
fn baseline_serialization_omits_runtime_authority_but_verify_json_keeps_it() -> Result<()> {
    let verifier = verify_benchmark_artifacts(BenchVerifyOptions::new(
        PathBuf::from("eval/public"),
        "eval/claims/registry.json",
    ))?;
    let verify_json = serde_json::to_value(&verifier)?;
    assert!(verify_json.get("authority_verdict").is_some());
    assert!(verify_json["authority_verdict"]
        .get("consumed_bytes")
        .is_some());

    let baseline = super::report::generate_public_baseline_report_from_verified(
        Path::new("eval/public"),
        verifier,
    )?;
    let baseline_json = serde_json::to_value(&baseline)?;
    let persisted_verifier = &baseline_json["artifact_verifier"];
    assert!(persisted_verifier.get("authority_verdict").is_none());
    assert!(!serde_json::to_string(&baseline_json)?.contains("consumed_bytes"));
    assert!(!baseline
        .artifact_verifier
        .authority_verdict
        .consumed_bytes
        .is_empty());
    Ok(())
}

#[test]
fn security_snapshot_validation_uses_consumed_bytes_after_source_removed() -> Result<()> {
    let root = copy_public_fixture("snapshot-consumed-bytes")?;
    let removed_snapshot = Arc::new(Mutex::new(None));
    let hook_result = Arc::clone(&removed_snapshot);

    crate::eval::bench_artifact::verify::security_snapshot::set_after_security_snapshot_consumed_hook(
        move |path| {
            let bytes = fs::read(path).expect("snapshot still exists at test hook");
            let hash = format!("{:x}", Sha256::digest(&bytes));
            fs::remove_file(path).expect("remove consumed snapshot source");
            *hook_result.lock().expect("lock hook result") = Some((path.to_path_buf(), hash));
        },
    );

    let report = verify_benchmark_artifacts(BenchVerifyOptions::new(
        root.clone(),
        "eval/claims/registry.json",
    ))?;
    let (removed_path, expected_hash) = removed_snapshot
        .lock()
        .expect("lock removed snapshot")
        .clone()
        .expect("snapshot consumption hook must run");
    let relative_path = removed_path
        .strip_prefix(&root)?
        .to_string_lossy()
        .into_owned();

    assert!(!removed_path.exists());
    assert!(report.passed, "{:#?}", report.failures);
    assert_eq!(
        report
            .authority_verdict
            .consumed_bytes
            .get(&relative_path),
        Some(&expected_hash)
    );
    Ok(())
}
