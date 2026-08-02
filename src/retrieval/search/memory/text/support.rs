//! Order-preserving helpers for the query search path, split out of
//! `text.rs` to keep it under the repository file-size ceiling.

use std::collections::{HashMap, HashSet};

use crate::memory::Memory;
use anyhow::Result;
use rusqlite::{types::ToSql, Connection};

use super::super::super::common::{
    calibrated_signal, reciprocal_rank_score, weighted_rank_score, WeightedRankedChannel,
};
#[cfg(test)]
use super::super::SearchWeights;
use super::super::{ChannelContribution, ChannelContributionBreakdown};
use super::{NamedChannel, QuerySearchPlan};

mod fact;
mod graph_claim;
use fact::structured_fact_scope;
pub(super) use graph_claim::resolve_graph_claim_support;

pub(super) fn log_search_timing(
    query_text: &str,
    project: Option<&str>,
    limit: i64,
    offset: i64,
    plan: &QuerySearchPlan,
) {
    crate::log::info(
        "search-perf",
        &format!(
            "query={} project={} limit={} offset={} fetch_limit={} {}",
            crate::db::truncate_str(query_text, 80),
            project.unwrap_or("-"),
            limit,
            offset,
            plan.fetch_limit,
            crate::perf::format_phase_timings(&plan.timings)
        ),
    );
}

pub(super) struct ContributionSet {
    pub totals: Vec<ChannelContribution>,
    pub breakdowns: Vec<ChannelContributionBreakdown>,
}

pub(super) fn contributions_for(memory_id: i64, plan: &QuerySearchPlan) -> Result<ContributionSet> {
    let mut totals = Vec::new();
    let mut breakdowns = Vec::new();
    for channel in &plan.channels {
        if channel.weight <= 0.0 {
            continue;
        }
        let Some(index) = channel.hits.iter().position(|hit| hit.id == memory_id) else {
            continue;
        };
        let normalized_score = calibrated_signal(channel.hits[index].normalized_score)?;
        let reciprocal_rank = reciprocal_rank_score(plan.weights.rrf_k, index)?;
        let total_score =
            weighted_rank_score(channel.weight, plan.weights.rrf_k, index, normalized_score)?;
        totals.push(ChannelContribution {
            channel: channel.name.to_string(),
            rank: index + 1,
            score: total_score,
        });
        breakdowns.push(ChannelContributionBreakdown {
            channel: channel.name.to_string(),
            rank: index + 1,
            weight: channel.weight,
            reciprocal_rank,
            normalized_signal: normalized_score,
            total_score,
        });
    }
    Ok(ContributionSet { totals, breakdowns })
}

pub(super) fn weighted_channel_inputs(channels: &[NamedChannel]) -> Vec<WeightedRankedChannel<'_>> {
    channels
        .iter()
        .filter(|channel| channel.has_hits())
        .map(|channel| WeightedRankedChannel {
            weight: channel.weight,
            hits: &channel.hits,
        })
        .collect()
}

pub(super) fn retrieved_candidate_ids(channels: &[NamedChannel]) -> Vec<i64> {
    let mut ids = channels
        .iter()
        .filter(|channel| channel.has_hits())
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub(super) fn resolve_explicit_entity_scope(
    conn: &Connection,
    query_text: &str,
    project: Option<&str>,
    fused: &[(i64, f64)],
    plan: &mut QuerySearchPlan,
    memories: &[Memory],
) -> Result<()> {
    let visible_ids = memories
        .iter()
        .map(|memory| memory.id)
        .collect::<HashSet<_>>();
    let mut fact_candidate_ids = plan
        .channels
        .iter()
        .filter(|channel| channel.name == "fact")
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .filter(|id| visible_ids.contains(id))
        .collect::<Vec<_>>();
    fact_candidate_ids.sort_unstable();
    fact_candidate_ids.dedup();

    let visible_fused_ids = fused
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| visible_ids.contains(id))
        .collect::<Vec<_>>();
    let visible_anchor_ids = plan
        .explicit_entity_memory_ids
        .iter()
        .copied()
        .filter(|id| visible_ids.contains(id))
        .collect::<Vec<_>>();
    let mut direct_ids =
        graph_claim::trusted_explicit_entity_anchor_ids(conn, project, plan, &visible_anchor_ids)?
            .into_iter()
            .collect::<HashSet<_>>();
    direct_ids.extend(
        memories
            .iter()
            .filter(|memory| memory_has_explicit_entity(memory, &plan.explicit_entity_terms))
            .map(|memory| memory.id),
    );

    let fact_scope = structured_fact_scope(
        conn,
        &fact_candidate_ids,
        query_text,
        &plan.explicit_entity_terms,
        &plan.claim_terms,
        plan.weights.min_evidence_confidence.clamp(0.0, 1.0),
        project,
        crate::retrieval::temporal::FactTimeMode::from_query(query_text),
    )?;
    direct_ids.extend(fact_scope.bound_ids);
    plan.fact_supported_memory_ids = fact_scope.supported_ids.into_iter().collect();
    plan.fact_supported_memory_ids.sort_unstable();

    if plan.explicit_entity_terms.is_empty() {
        plan.explicit_entity_memory_ids.clear();
        plan.explicit_entity_neighbor_ids.clear();
        return Ok(());
    }

    plan.explicit_entity_memory_ids = visible_fused_ids
        .iter()
        .copied()
        .filter(|id| direct_ids.contains(id))
        .collect();
    plan.explicit_entity_neighbor_ids = specific_entity_bridge_ids(
        conn,
        &plan.explicit_entity_memory_ids,
        &visible_fused_ids,
        memories,
        project,
        &plan.explicit_entity_terms,
    )?;
    Ok(())
}

fn specific_entity_bridge_ids(
    conn: &Connection,
    anchor_ids: &[i64],
    candidate_ids: &[i64],
    memories: &[Memory],
    project: Option<&str>,
    explicit_entity_terms: &[String],
) -> Result<Vec<i64>> {
    if anchor_ids.is_empty() || candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (1..=candidate_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>();
    let sql = format!(
        "SELECT me.memory_id, e.id, e.canonical_name
         FROM memory_entities me
         JOIN entities e ON e.id = me.entity_id
         WHERE me.memory_id IN ({})
         ORDER BY me.memory_id, e.id",
        placeholders.join(", ")
    );
    let params = candidate_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn ToSql>)
        .collect::<Vec<_>>();
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let memory_by_id = memories
        .iter()
        .map(|memory| (memory.id, memory))
        .collect::<HashMap<_, _>>();
    let mut excluded_terms = super::super::claim::project_entity_terms(project);
    excluded_terms.extend(
        explicit_entity_terms
            .iter()
            .map(|term| term.trim().to_lowercase()),
    );
    let mut entities_by_memory = HashMap::<i64, HashSet<i64>>::new();
    for row in rows {
        let (memory_id, entity_id, entity_name) = row?;
        if memory_by_id
            .get(&memory_id)
            .is_some_and(|memory| is_concrete_bridge_entity(&entity_name, memory, &excluded_terms))
        {
            entities_by_memory
                .entry(memory_id)
                .or_default()
                .insert(entity_id);
        }
    }
    let anchor_set = anchor_ids.iter().copied().collect::<HashSet<_>>();
    let anchor_entities = anchor_ids
        .iter()
        .filter_map(|id| entities_by_memory.get(id))
        .flat_map(|ids| ids.iter().copied())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    Ok(candidate_ids
        .iter()
        .copied()
        .filter(|id| !anchor_set.contains(id) && seen.insert(*id))
        .filter(|id| {
            entities_by_memory
                .get(id)
                .is_some_and(|ids| ids.iter().any(|entity| anchor_entities.contains(entity)))
        })
        .collect())
}

fn is_concrete_bridge_entity(
    entity_name: &str,
    memory: &Memory,
    excluded_terms: &HashSet<String>,
) -> bool {
    let normalized = entity_name.trim().to_lowercase();
    if normalized.is_empty()
        || excluded_terms.contains(&normalized)
        || matches!(
            normalized.as_str(),
            "aider"
                | "api"
                | "axum"
                | "claude"
                | "codex"
                | "cursor"
                | "engram"
                | "fts5"
                | "hindsight"
                | "hook"
                | "letta"
                | "mcp"
                | "mem0"
                | "owner"
                | "pager"
                | "project"
                | "remem"
                | "rest"
                | "sqlcipher"
                | "sqlite"
                | "team"
                | "tokio"
                | "tooladapter"
                | "trigram"
                | "zep"
        )
    {
        return false;
    }
    if super::super::claim::has_distinctive_entity_shape(entity_name) {
        return true;
    }
    ["team", "squad", "crew"].iter().any(|designator| {
        let phrase = format!("{designator} {}", entity_name.trim());
        super::super::claim::text_contains_exact_token(&memory.title, &phrase)
            || super::super::claim::text_contains_exact_token(&memory.text, &phrase)
    })
}

pub(super) fn apply_confidence_gate(
    fused: &[(i64, f64)],
    plan: &QuerySearchPlan,
    memories: &[Memory],
) -> Vec<(i64, f64)> {
    let min_confidence = plan.weights.min_evidence_confidence.clamp(0.0, 1.0);
    if min_confidence <= 0.0 {
        return fused.to_vec();
    }
    let memory_by_id: HashMap<i64, &Memory> =
        memories.iter().map(|memory| (memory.id, memory)).collect();
    let has_grounded_survivor = fused.iter().any(|(memory_id, _score)| {
        has_grounded_channel(*memory_id, plan)
            && memory_by_id.get(memory_id).is_some_and(|memory| {
                candidate_in_explicit_entity_scope(memory, plan)
                    && grounded_confidence(memory, plan) >= min_confidence
            })
    });
    fused
        .iter()
        .copied()
        .filter(|(memory_id, _score)| {
            memory_by_id.get(memory_id).is_some_and(|memory| {
                if !candidate_in_explicit_entity_scope(memory, plan) {
                    return false;
                }
                if plan.claim_terms.is_empty()
                    || has_fact_evidence(*memory_id, plan)
                    || has_graph_claim_evidence(*memory_id, plan)
                {
                    return true;
                }
                let confidence = grounded_confidence(memory, plan);
                if confidence >= min_confidence {
                    return true;
                }
                is_vector_only(*memory_id, plan)
                    && !has_grounded_survivor
                    && plan.explicit_entity_terms.is_empty()
            })
        })
        .collect()
}

pub(super) fn candidate_confidence(memory: &Memory, plan: &QuerySearchPlan) -> f64 {
    if !candidate_in_explicit_entity_scope(memory, plan) {
        return 0.0;
    }
    if plan.claim_terms.is_empty()
        || has_fact_evidence(memory.id, plan)
        || has_graph_claim_evidence(memory.id, plan)
        || is_vector_only(memory.id, plan)
    {
        return 1.0;
    }
    candidate_claim_confidence(memory, plan)
}

fn grounded_confidence(memory: &Memory, plan: &QuerySearchPlan) -> f64 {
    if has_fact_evidence(memory.id, plan) || has_graph_claim_evidence(memory.id, plan) {
        1.0
    } else {
        candidate_claim_confidence(memory, plan)
    }
}

fn has_fact_evidence(memory_id: i64, plan: &QuerySearchPlan) -> bool {
    plan.fact_supported_memory_ids.contains(&memory_id)
        && plan.channels.iter().any(|channel| {
            channel.name == "fact" && channel.hits.iter().any(|hit| hit.id == memory_id)
        })
}

fn has_graph_claim_evidence(memory_id: i64, plan: &QuerySearchPlan) -> bool {
    plan.graph_claim_supported_memory_ids.contains(&memory_id)
        && plan.channels.iter().any(|channel| {
            channel.name == "graph_traversal" && channel.hits.iter().any(|hit| hit.id == memory_id)
        })
}

fn candidate_claim_confidence(memory: &Memory, plan: &QuerySearchPlan) -> f64 {
    super::super::claim::claim_term_coverage(memory, &plan.claim_terms)
}

fn candidate_in_explicit_entity_scope(memory: &Memory, plan: &QuerySearchPlan) -> bool {
    plan.explicit_entity_terms.is_empty()
        || plan.explicit_entity_memory_ids.contains(&memory.id)
        || plan.explicit_entity_neighbor_ids.contains(&memory.id)
        || memory_has_explicit_entity(memory, &plan.explicit_entity_terms)
}

fn memory_has_explicit_entity(memory: &Memory, explicit_entity_terms: &[String]) -> bool {
    explicit_entity_terms.iter().all(|term| {
        super::super::claim::text_contains_exact_token(&memory.title, term)
            || super::super::claim::text_contains_exact_token(&memory.text, term)
    })
}

fn has_grounded_channel(memory_id: i64, plan: &QuerySearchPlan) -> bool {
    plan.channels.iter().any(|channel| {
        !matches!(channel.name, "vector" | "usage")
            && channel.hits.iter().any(|hit| hit.id == memory_id)
    })
}

fn is_vector_only(memory_id: i64, plan: &QuerySearchPlan) -> bool {
    let mut contributing = plan
        .channels
        .iter()
        .filter(|channel| channel.hits.iter().any(|hit| hit.id == memory_id))
        .map(|channel| channel.name)
        .filter(|name| *name != "usage");
    contributing
        .next()
        .is_some_and(|first| first == "vector" && contributing.all(|name| name == "vector"))
}

pub(super) fn visibility_label(memory: &Memory, requested_project: Option<&str>) -> &'static str {
    if memory.scope == "global" {
        "global-overlay"
    } else if requested_project
        .map(|project| crate::project_id::project_matches(Some(&memory.project), project))
        .unwrap_or(false)
    {
        "project-local"
    } else {
        "unscoped"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::search::common::WeightedRankedHit;

    fn plan_with_channels(channels: Vec<NamedChannel>) -> QuerySearchPlan {
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

    fn channel(name: &'static str, ids: &[i64]) -> NamedChannel {
        NamedChannel::enabled_with_hits(
            name,
            1.0,
            ids.iter()
                .copied()
                .map(|id| WeightedRankedHit::scored(id, 1.0))
                .collect(),
        )
    }

    #[test]
    fn rank_only_contributions_explain_pure_rrf_components() -> Result<()> {
        let plan = plan_with_channels(vec![
            NamedChannel::enabled("entity", 1.0, vec![1, 2]),
            NamedChannel::enabled("temporal", 0.1, vec![2]),
        ]);

        let contributions = contributions_for(2, &plan)?;
        assert_eq!(contributions.totals.len(), 2);
        assert_eq!(contributions.breakdowns.len(), 2);
        let entity = &contributions.totals[0];
        assert_eq!(entity.channel, "entity");
        assert_eq!(entity.rank, 2);
        assert_eq!(entity.score, reciprocal_rank_score(60.0, 1)?);

        let temporal = &contributions.totals[1];
        assert_eq!(temporal.channel, "temporal");
        assert_eq!(temporal.rank, 1);
        assert_eq!(temporal.score, 0.1 * reciprocal_rank_score(60.0, 0)?);
        let entity_breakdown = &contributions.breakdowns[0];
        assert_eq!(entity_breakdown.channel, "entity");
        assert_eq!(entity_breakdown.rank, 2);
        assert_eq!(entity_breakdown.weight, 1.0);
        assert_eq!(
            entity_breakdown.reciprocal_rank,
            reciprocal_rank_score(60.0, 1)?
        );
        assert_eq!(entity_breakdown.normalized_signal, None);
        assert_eq!(entity_breakdown.total_score, entity.score);
        Ok(())
    }

    #[test]
    fn calibrated_zero_signal_remains_distinct_from_rank_only() -> Result<()> {
        let plan = plan_with_channels(vec![NamedChannel::enabled_with_hits(
            "fts",
            2.5,
            vec![WeightedRankedHit::scored(1, 0.0)],
        )]);

        let contributions = contributions_for(1, &plan)?;
        assert_eq!(contributions.breakdowns.len(), 1);
        let breakdown = &contributions.breakdowns[0];
        assert_eq!(breakdown.normalized_signal, Some(0.0));
        assert_eq!(
            breakdown.total_score,
            breakdown.weight * breakdown.reciprocal_rank
        );
        Ok(())
    }

    fn memory(id: i64, text: &str) -> Memory {
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
    fn weak_grounded_hits_do_not_suppress_vector_semantic_fallback() {
        let plan = plan_with_channels(vec![channel("fts", &[1]), channel("vector", &[2])]);
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert_eq!(
            apply_confidence_gate(&fused, &plan, &[memory(1, "weak"), memory(2, "semantic")]),
            vec![(2, 0.5)]
        );
    }

    #[test]
    fn qualified_grounded_survivor_suppresses_vector_only_tail() {
        let plan = plan_with_channels(vec![channel("fts", &[1]), channel("vector", &[1, 2])]);
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert_eq!(
            apply_confidence_gate(
                &fused,
                &plan,
                &[memory(1, "supported unsupported claim"), memory(2, "tail")]
            ),
            vec![(1, 1.0)]
        );
    }

    #[test]
    fn vector_only_candidate_with_claim_support_survives_other_grounded_result() {
        let plan = plan_with_channels(vec![channel("fts", &[1]), channel("vector", &[2])]);
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert_eq!(
            apply_confidence_gate(
                &fused,
                &plan,
                &[
                    memory(1, "unsupported grounded"),
                    memory(2, "unsupported semantic")
                ]
            ),
            fused
        );
    }

    #[test]
    fn explicit_entity_anchor_suppresses_unsupported_vector_fallback() {
        let mut plan = plan_with_channels(vec![channel("fts", &[1]), channel("vector", &[2])]);
        plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
        plan.explicit_entity_memory_ids = vec![1];
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert!(
            apply_confidence_gate(&fused, &plan, &[memory(1, "weak"), memory(2, "semantic")])
                .is_empty()
        );
    }

    #[test]
    fn entity_bound_vector_candidate_still_requires_claim_support() {
        let mut plan = plan_with_channels(vec![channel("fts", &[1]), channel("vector", &[2])]);
        plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
        plan.explicit_entity_memory_ids = vec![2];
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert!(
            apply_confidence_gate(&fused, &plan, &[memory(1, "weak"), memory(2, "semantic")])
                .is_empty()
        );
        assert_eq!(
            apply_confidence_gate(
                &fused,
                &plan,
                &[memory(1, "weak"), memory(2, "unsupported semantic")]
            ),
            vec![(2, 0.5)]
        );
    }

    #[test]
    fn entity_neighbor_with_claim_support_survives_multi_hop_gate() {
        let mut plan = plan_with_channels(vec![channel("fts", &[1, 2])]);
        plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
        plan.explicit_entity_memory_ids = vec![1];
        plan.explicit_entity_neighbor_ids = vec![2];
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert_eq!(
            apply_confidence_gate(
                &fused,
                &plan,
                &[
                    memory(1, "NebulaLatch unsupported"),
                    memory(2, "Team Mica unsupported")
                ]
            ),
            fused
        );
    }

    #[test]
    fn entity_neighbor_without_claim_support_is_rejected() {
        let mut plan = plan_with_channels(vec![channel("fts", &[1, 2])]);
        plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];
        plan.explicit_entity_memory_ids = vec![1];
        plan.explicit_entity_neighbor_ids = vec![2];
        let fused = vec![(1, 1.0), (2, 0.5)];

        assert_eq!(
            apply_confidence_gate(
                &fused,
                &plan,
                &[
                    memory(1, "NebulaLatch unsupported"),
                    memory(2, "Team Mica pager")
                ]
            ),
            vec![(1, 1.0)]
        );
    }

    #[test]
    fn unknown_explicit_entity_suppresses_unbound_vector_fallback() {
        let mut plan = plan_with_channels(vec![channel("vector", &[2])]);
        plan.explicit_entity_terms = vec!["UnknownLatch".to_string()];

        assert!(apply_confidence_gate(&[(2, 1.0)], &plan, &[memory(2, "semantic")]).is_empty());
    }

    #[test]
    fn exact_entity_token_in_memory_is_a_binding_fallback() {
        let mut plan = plan_with_channels(vec![channel("vector", &[2])]);
        plan.explicit_entity_terms = vec!["NebulaLatch".to_string()];

        assert_eq!(
            apply_confidence_gate(
                &[(2, 1.0)],
                &plan,
                &[memory(2, "NebulaLatch unsupported semantic")]
            ),
            vec![(2, 1.0)]
        );
    }

    #[test]
    fn vector_only_query_still_trusts_semantic_recall() {
        let plan = plan_with_channels(vec![channel("vector", &[2]), channel("usage", &[2])]);

        assert_eq!(
            apply_confidence_gate(&[(2, 1.0)], &plan, &[memory(2, "semantic")]),
            vec![(2, 1.0)]
        );
    }
}

#[cfg(test)]
#[path = "support_scope_tests.rs"]
mod scope_tests;

#[cfg(test)]
#[path = "support_fact_tests.rs"]
mod fact_tests;
