use std::collections::{HashMap, HashSet};

use super::super::memory_selection::sort_memories_by_branch;
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
fn index_render_applies_item_limit_before_type_grouping() {
    let mut output = String::new();
    let limits = ContextLimits {
        memory_index_limit: 1,
        ..ContextLimits::default()
    };
    let memories = vec![
        sample_memory_with_epoch(2, "discovery", "Selected discovery", REF_EPOCH),
        sample_memory_with_epoch(1, "decision", "Unselected decision", REF_EPOCH),
    ];

    render_memory_index_with_limits_excluding_and_staleness(
        &mut output,
        &memories,
        &limits,
        &HashSet::new(),
        REF_EPOCH,
        &HashMap::new(),
    );

    assert!(output.contains("Selected discovery"), "{output}");
    assert!(!output.contains("Unselected decision"), "{output}");
}

#[test]
fn branch_sort_preserves_retrieval_order_within_branch_bucket() {
    let mut memories = vec![
        {
            let mut memory = sample_memory_with_epoch(1, "decision", "Older relevant", REF_EPOCH);
            memory.branch = Some("main".to_string());
            memory
        },
        {
            let mut memory =
                sample_memory_with_epoch(2, "decision", "Newer less relevant", REF_EPOCH + 100);
            memory.branch = Some("main".to_string());
            memory
        },
        sample_memory_with_epoch(3, "decision", "Global fallback", REF_EPOCH + 200),
    ];

    sort_memories_by_branch(&mut memories, Some("main"));

    let ids = memories.iter().map(|memory| memory.id).collect::<Vec<_>>();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[test]
fn core_render_preserves_retrieval_order_for_equal_score_bucket() {
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
    assert!(second_pos < first_pos, "{output}");
}
