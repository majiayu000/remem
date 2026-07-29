use super::*;
use crate::retrieval::search::common::WeightedRankedHit;

fn scope_plan(channels: Vec<NamedChannel>) -> QuerySearchPlan {
    QuerySearchPlan {
        expanded_terms: vec![],
        core_terms: vec![],
        claim_terms: vec!["unsupported".to_string()],
        explicit_entity_terms: vec![],
        explicit_entity_memory_ids: vec![],
        explicit_entity_neighbor_ids: vec![],
        fact_supported_memory_ids: channels
            .iter()
            .filter(|channel| channel.name == "fact")
            .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
            .collect(),
        graph_claim_supported_memory_ids: vec![],
        memory_type: None,
        branch: None,
        include_stale: false,
        fts_query: None,
        temporal_range: None,
        temporal_field: None,
        include_suppressed: false,
        fetch_limit: 10,
        weights: SearchWeights::default(),
        channels,
        timings: vec![],
    }
}

fn scope_channel(name: &'static str, ids: &[i64]) -> NamedChannel {
    NamedChannel::enabled_with_hits(
        name,
        1.0,
        ids.iter()
            .copied()
            .map(|id| WeightedRankedHit {
                id,
                normalized_score: 1.0,
            })
            .collect(),
    )
}

fn scope_memory(id: i64, text: &str) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "/repo".to_string(),
        topic_key: None,
        title: String::new(),
        text: text.to_string(),
        memory_type: "decision".to_string(),
        files: None,
        created_at_epoch: 1,
        updated_at_epoch: 1,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

#[test]
fn fact_evidence_remains_trusted_but_graph_requires_claim_support() {
    let fact_plan = scope_plan(vec![
        scope_channel("fact", &[1]),
        scope_channel("vector", &[1, 2]),
    ]);
    let graph_plan = scope_plan(vec![
        scope_channel("fts", &[1]),
        scope_channel("graph_traversal", &[2]),
        scope_channel("vector", &[2]),
    ]);

    assert_eq!(
        candidate_confidence(&scope_memory(1, "weak"), &fact_plan),
        1.0
    );
    assert_eq!(
        candidate_confidence(&scope_memory(2, "weak"), &graph_plan),
        0.0
    );
}

#[test]
fn bridged_graph_candidate_requires_claim_support() {
    let mut plan = scope_plan(vec![scope_channel("graph_traversal", &[2])]);
    plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
    plan.explicit_entity_neighbor_ids = vec![2];

    assert!(apply_confidence_gate(&[(2, 1.0)], &plan, &[scope_memory(2, "bound")]).is_empty());
    assert_eq!(
        apply_confidence_gate(
            &[(2, 1.0)],
            &plan,
            &[scope_memory(2, "bound unsupported claim")]
        ),
        vec![(2, 1.0)]
    );
}

#[test]
fn trusted_graph_path_must_carry_explicit_source_claim_support() {
    let mut plan = scope_plan(vec![scope_channel("graph_traversal", &[2])]);
    assert_eq!(
        candidate_confidence(&scope_memory(2, "unrelated target"), &plan),
        0.0
    );

    plan.graph_claim_supported_memory_ids = vec![2];
    assert_eq!(
        candidate_confidence(&scope_memory(2, "unrelated target"), &plan),
        1.0
    );

    plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
    assert_eq!(
        candidate_confidence(&scope_memory(2, "unrelated target"), &plan),
        0.0,
        "inherited graph claims must not bypass explicit entity scope"
    );
}

#[test]
fn fact_and_empty_claim_cannot_bypass_explicit_entity_scope() {
    let mut plan = scope_plan(vec![scope_channel("fact", &[2])]);
    plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
    assert!(apply_confidence_gate(&[(2, 1.0)], &plan, &[scope_memory(2, "unbound")]).is_empty());

    plan.explicit_entity_neighbor_ids = vec![2];
    assert_eq!(
        apply_confidence_gate(&[(2, 1.0)], &plan, &[scope_memory(2, "bound")]),
        vec![(2, 1.0)]
    );

    plan.claim_terms.clear();
    plan.explicit_entity_neighbor_ids.clear();
    assert!(apply_confidence_gate(&[(2, 1.0)], &plan, &[scope_memory(2, "unbound")]).is_empty());
}

#[test]
fn fused_bridge_uses_specific_entities_not_common_tags() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    crate::retrieval::entity::link_entities(
        &conn,
        1,
        &["NebulaLatch", "Team", "Mica", "API", "remem", "Claude"].map(str::to_string),
    )?;
    crate::retrieval::entity::link_entities(&conn, 2, &["Team", "Mica"].map(str::to_string))?;
    crate::retrieval::entity::link_entities(
        &conn,
        3,
        &["Team", "API", "remem", "Claude"].map(str::to_string),
    )?;
    crate::retrieval::entity::link_entities(&conn, 4, &["Mica".to_string()])?;
    let memories = vec![
        scope_memory(1, "NebulaLatch is owned by Team Mica"),
        scope_memory(2, "Team Mica uses pager mica-17"),
        scope_memory(3, "Team API supports remem for Claude"),
    ];

    assert_eq!(
        specific_entity_bridge_ids(
            &conn,
            &[1],
            &[1, 2, 3],
            &memories,
            Some("/repo"),
            &["NebulaLatch".to_string()],
        )?,
        vec![2]
    );
    Ok(())
}
