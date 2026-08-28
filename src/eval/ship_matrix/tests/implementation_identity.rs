use super::super::source_is_clean;
use std::process::Command;

#[test]
fn dirty_build_cannot_become_source_clean_after_checkout_is_reverted() {
    assert!(!source_is_clean(Some(true), Some(false)));
    assert!(!source_is_clean(None, Some(false)));
    assert!(source_is_clean(Some(false), Some(false)));
}

#[test]
fn build_script_watches_the_benchmark_authority_suite() {
    let build_script = include_str!("../../../../build.rs");
    let contract = include_str!("../../../../eval/production-input-pathspec-v1.json");
    assert!(build_script.contains("production-input-pathspec-v1.json"));
    assert!(contract.contains("eval/public/memory/suites/adversarial-policy/suite.json"));
    assert!(!build_script.contains(":(exclude)src/eval/ship_matrix"));
    assert!(!build_script.contains(":(exclude)src/eval/gates.rs"));
}

#[test]
fn production_identity_implementations_share_machine_contract() {
    let build_script = include_str!("../../../../build.rs");
    let rust_authority = include_str!("../authority.rs");
    let python_guard = include_str!("../../../../scripts/ci/check_public_claims.py");
    for implementation in [build_script, rust_authority, python_guard] {
        assert!(implementation.contains("production-input-pathspec-v1.json"));
        assert!(!implementation.contains("\"Cargo.lock\""));
    }

    let output = Command::new("python3")
        .args([
            "scripts/ci/check_public_claims.py",
            "--print-production-input-tree",
        ])
        .output()
        .expect("run Python production identity guard");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let python_tree = String::from_utf8(output.stdout).expect("Python tree is UTF-8");
    assert_eq!(
        python_tree.trim(),
        super::super::authority::production_input_tree_sha256()
            .expect("Rust production tree identity")
    );
}
