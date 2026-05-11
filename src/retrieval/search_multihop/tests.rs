use crate::memory::Memory;

use super::discover::discover_entities;
use super::merge::rank_merged_ids;

fn make_memory(id: i64, title: &str, text: &str) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "proj".to_string(),
        topic_key: None,
        title: title.to_string(),
        text: text.to_string(),
        memory_type: "discovery".to_string(),
        files: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

#[test]
fn discover_entities_skips_query_entities_and_deduplicates() {
    let first_hop = vec![
        make_memory(1, "Melanie", "Tom Sarah"),
        make_memory(2, "Tom", "Sarah Tom"),
    ];

    let entities = discover_entities("Melanie", &first_hop);
    assert_eq!(entities, vec!["Tom", "Sarah"]);
}

#[test]
fn rank_merged_ids_boosts_overlap_and_respects_limit() {
    let ranked = rank_merged_ids(&[1, 2], &[2, 3], 3);
    assert_eq!(ranked, vec![2, 1, 3]);

    let limited = rank_merged_ids(&[1, 2], &[2, 3], 2);
    assert_eq!(limited, vec![2, 1]);
}
