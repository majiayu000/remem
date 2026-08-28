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
    assert!(build_script.contains("eval/public/memory/suites/adversarial-policy/suite.json"));
    assert!(!build_script.contains(":(exclude)src/eval/ship_matrix"));
    assert!(!build_script.contains(":(exclude)src/eval/gates.rs"));
}
