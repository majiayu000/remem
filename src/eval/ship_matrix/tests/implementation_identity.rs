use super::super::source_is_clean;

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
    assert!(contract.contains("eval/coding-bench/fixtures/tasks.json"));
    assert!(contract.contains("eval/claims/registry.json"));
    assert!(!build_script.contains(":(exclude)src/eval/ship_matrix"));
    assert!(!build_script.contains(":(exclude)src/eval/gates.rs"));
}

#[test]
fn production_identity_implementations_share_machine_contract() {
    let build_script = include_str!("../../../../build.rs");
    let rust_authority = include_str!("../../bench_artifact/authority.rs");
    for implementation in [build_script, rust_authority] {
        assert!(implementation.contains("production-input-pathspec-v1.json"));
        assert!(!implementation.contains("\"Cargo.lock\""));
    }
    let ship_matrix = include_str!("../../ship_matrix.rs");
    assert!(!ship_matrix.contains("option_env!"));
    assert!(!ship_matrix.contains("production_input_tree_sha256("));
}

#[test]
fn native_workflow_tracks_every_production_input_root() {
    let workflow = include_str!("../../../../.github/workflows/native-benchmark-evidence.yml");
    for trigger in [
        ".cargo/**",
        "assets/**",
        "prompts/**",
        "eval/coding-bench/fixtures/tasks.json",
        "eval/claims/**",
    ] {
        assert!(
            workflow.contains(&format!("- {trigger}")),
            "native evidence trigger is missing production input {trigger}"
        );
    }
}

#[test]
fn native_workflow_uses_the_release_feature_profile() {
    let workflow = include_str!("../../../../.github/workflows/native-benchmark-evidence.yml");
    assert!(workflow.contains("cargo_flags: --no-default-features --features eval"));
    assert_eq!(workflow.matches("cargo_flags: \"\"").count(), 3);
    assert_eq!(
        workflow
            .matches("cargo run --locked ${{ matrix.cargo_flags }} --")
            .count(),
        2
    );
    assert!(workflow.contains("cargo run --locked -- bench verify --root \"$PUBLIC_ROOT\""));
    assert!(workflow.contains("Authenticate downloaded native row receipts"));
    assert!(workflow.contains("row-verifier-*.json"));
    assert!(workflow.contains("seen_targets"));
}

#[test]
fn crate_package_excludes_public_sqlite_snapshots() {
    let manifest = include_str!("../../../../Cargo.toml");
    assert!(manifest.contains("\"eval/public/**/*.sqlite3\""));
}
