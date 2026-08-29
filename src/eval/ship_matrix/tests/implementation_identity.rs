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
