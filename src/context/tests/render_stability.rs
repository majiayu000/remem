use std::collections::{HashMap, HashSet};

use super::super::policy::ContextLimits;
use super::super::sections::{
    render_core_memory_with_limits_and_staleness,
    render_memory_index_with_limits_excluding_and_staleness,
};
use super::sample_memory_with_epoch;

const REF_EPOCH: i64 = 1_710_000_000;

#[test]
fn core_render_uses_input_reference_epoch_for_staleness() {
    let mut output = String::new();
    let memory =
        sample_memory_with_epoch(1, "decision", "Boundary decision", REF_EPOCH - 31 * 86_400);

    render_core_memory_with_limits_and_staleness(
        &mut output,
        &[memory],
        &ContextLimits::default(),
        REF_EPOCH,
        &HashMap::new(),
    );

    assert!(output.contains("staleness=aging"), "{output}");
}

#[test]
fn index_render_uses_stable_tiebreaks_for_equal_timestamp_items() {
    let mut output = String::new();
    let memories = vec![
        sample_memory_with_epoch(2, "decision", "Second decision", REF_EPOCH),
        sample_memory_with_epoch(1, "decision", "First decision", REF_EPOCH),
    ];

    render_memory_index_with_limits_excluding_and_staleness(
        &mut output,
        &memories,
        &ContextLimits::default(),
        &HashSet::new(),
        REF_EPOCH,
        &HashMap::new(),
    );

    let first_pos = output.find("#1 First decision").unwrap();
    let second_pos = output.find("#2 Second decision").unwrap();
    assert!(first_pos < second_pos, "{output}");
}

#[test]
fn core_render_uses_stable_id_tiebreak_for_equal_score_bucket() {
    let mut output = String::new();
    let memories = vec![
        sample_memory_with_epoch(2, "decision", "Second decision", REF_EPOCH),
        sample_memory_with_epoch(1, "decision", "First decision", REF_EPOCH),
    ];

    render_core_memory_with_limits_and_staleness(
        &mut output,
        &memories,
        &ContextLimits::default(),
        REF_EPOCH,
        &HashMap::new(),
    );

    let first_pos = output.find("**#1 First decision**").unwrap();
    let second_pos = output.find("**#2 Second decision**").unwrap();
    assert!(first_pos < second_pos, "{output}");
}
