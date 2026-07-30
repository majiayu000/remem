use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::Connection;

use super::super::{NamedChannel, QuerySearchPlan};

const MIN_GRAPH_SEED_CLAIM_COUNT: usize = 2;
const MAX_GRAPH_SEEDS: usize = 32;

pub(in crate::retrieval::search::memory::text) fn trusted_explicit_entity_anchor_ids(
    conn: &Connection,
    project: Option<&str>,
    plan: &QuerySearchPlan,
    anchor_ids: &[i64],
) -> Result<Vec<i64>> {
    if plan.explicit_entity_terms.is_empty() || anchor_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut seen = HashSet::new();
    let ordered_anchor_ids = anchor_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect::<Vec<_>>();
    let anchor_set = ordered_anchor_ids.iter().copied().collect::<HashSet<_>>();
    let mut entity_anchor_groups = Vec::with_capacity(plan.explicit_entity_terms.len());
    for term in &plan.explicit_entity_terms {
        let ids = crate::retrieval::entity::search_exact_entity_names_filtered(
            conn,
            std::slice::from_ref(term),
            project,
            plan.memory_type.as_deref(),
            plan.branch.as_deref(),
            plan.fetch_limit,
            plan.include_stale,
        )?;
        let ids = super::super::super::suppression_filter::ids(conn, ids, plan.include_suppressed)?;
        let group = ids
            .into_iter()
            .filter(|id| anchor_set.contains(id))
            .collect::<HashSet<_>>();
        if group.is_empty() {
            return Ok(Vec::new());
        }
        entity_anchor_groups.push(group);
    }

    if entity_anchor_groups.len() == 1 {
        return Ok(ordered_anchor_ids
            .into_iter()
            .filter(|id| entity_anchor_groups[0].contains(id))
            .collect());
    }

    let connected = connected_anchor_components(
        conn,
        project,
        plan,
        &ordered_anchor_ids,
        &anchor_set,
        &entity_anchor_groups,
    )?;
    Ok(ordered_anchor_ids
        .into_iter()
        .filter(|id| connected.contains(id))
        .collect())
}

fn connected_anchor_components(
    conn: &Connection,
    project: Option<&str>,
    plan: &QuerySearchPlan,
    ordered_anchor_ids: &[i64],
    anchor_set: &HashSet<i64>,
    entity_anchor_groups: &[HashSet<i64>],
) -> Result<HashSet<i64>> {
    let mut adjacency = ordered_anchor_ids
        .iter()
        .copied()
        .map(|id| (id, HashSet::new()))
        .collect::<HashMap<_, _>>();
    let min_confidence = plan.weights.min_evidence_confidence.clamp(0.0, 1.0);
    let reference_time_epoch = chrono::Utc::now().timestamp();
    for anchor_id in ordered_anchor_ids {
        let seed_ids = [*anchor_id];
        let outcome = crate::retrieval::graph::traverse_trusted_graph(
            conn,
            crate::retrieval::graph::GraphTraversalRequest {
                seed_memory_ids: &seed_ids,
                project,
                memory_type: plan.memory_type.as_deref(),
                branch: plan.branch.as_deref(),
                include_inactive: plan.include_stale,
                reference_time_epoch,
                limits: crate::retrieval::graph::GraphTraversalLimits::for_search(plan.fetch_limit),
            },
        )?;
        for hit in outcome.hits {
            if hit.min_confidence < min_confidence || !anchor_set.contains(&hit.memory_id) {
                continue;
            }
            adjacency
                .entry(*anchor_id)
                .or_default()
                .insert(hit.memory_id);
            adjacency
                .entry(hit.memory_id)
                .or_default()
                .insert(*anchor_id);
        }
    }

    let mut connected = HashSet::new();
    let mut visited = HashSet::new();
    for start in ordered_anchor_ids {
        if visited.contains(start) {
            continue;
        }
        let mut component = HashSet::new();
        let mut stack = vec![*start];
        while let Some(current) = stack.pop() {
            if !component.insert(current) {
                continue;
            }
            visited.insert(current);
            if let Some(neighbors) = adjacency.get(&current) {
                stack.extend(neighbors.iter().copied());
            }
        }
        if entity_anchor_groups
            .iter()
            .all(|group| !group.is_disjoint(&component))
        {
            connected.extend(component);
        }
    }
    Ok(connected)
}

pub(in crate::retrieval::search::memory::text) fn resolve_graph_claim_support(
    conn: &Connection,
    project: Option<&str>,
    plan: &mut QuerySearchPlan,
) -> Result<()> {
    plan.graph_claim_supported_memory_ids.clear();
    if !plan.explicit_entity_terms.is_empty() || plan.claim_terms.is_empty() {
        return Ok(());
    }

    let graph_target_ids = channel_ids(&plan.channels, "graph_traversal");
    let fts_seed_ids = ordered_channel_ids(&plan.channels, "fts");
    if graph_target_ids.is_empty() || fts_seed_ids.is_empty() {
        return Ok(());
    }

    let supported_seed_ids = claim_supported_seed_ids(conn, project, plan, &fts_seed_ids)?;
    if supported_seed_ids.is_empty() {
        return Ok(());
    }
    let outcome = crate::retrieval::graph::traverse_trusted_graph(
        conn,
        crate::retrieval::graph::GraphTraversalRequest {
            seed_memory_ids: &supported_seed_ids,
            project,
            memory_type: plan.memory_type.as_deref(),
            branch: plan.branch.as_deref(),
            include_inactive: plan.include_stale,
            reference_time_epoch: chrono::Utc::now().timestamp(),
            limits: crate::retrieval::graph::GraphTraversalLimits::for_search(plan.fetch_limit),
        },
    )?;
    let min_edge_confidence = plan.weights.min_evidence_confidence.clamp(0.0, 1.0);
    // A claim-grounded FTS seed authorizes this query-level traversal, while the
    // graph channel still supplies the trusted, scoped, confidence-checked targets.
    // Explicit entity queries never enter this path.
    plan.graph_claim_supported_memory_ids = outcome
        .hits
        .into_iter()
        .filter(|hit| graph_target_ids.contains(&hit.memory_id))
        .filter(|hit| hit.min_confidence >= min_edge_confidence)
        .map(|hit| hit.memory_id)
        .collect();
    plan.graph_claim_supported_memory_ids.sort_unstable();
    plan.graph_claim_supported_memory_ids.dedup();
    Ok(())
}

fn claim_supported_seed_ids(
    conn: &Connection,
    project: Option<&str>,
    plan: &QuerySearchPlan,
    fts_seed_ids: &[i64],
) -> Result<Vec<i64>> {
    let memories = crate::memory::get_memories_by_ids_with_suppressed_policy(
        conn,
        fts_seed_ids,
        project,
        plan.include_suppressed,
    )?;
    let supported = memories
        .into_iter()
        .filter(|memory| {
            let text = format!("{} {}", memory.title, memory.text);
            graph_seed_supports_claim(&text, &plan.claim_terms)
        })
        .map(|memory| memory.id)
        .collect::<HashSet<_>>();
    Ok(bounded_supported_seed_ids(fts_seed_ids, &supported))
}

fn graph_seed_supports_claim(text: &str, claim_terms: &[String]) -> bool {
    let required_terms = claim_terms
        .iter()
        .filter(|term| !super::super::super::claim::is_nonsemantic_claim_modifier(term))
        .filter(|term| !is_graph_query_scaffold(term))
        .collect::<Vec<_>>();
    required_terms.len() >= MIN_GRAPH_SEED_CLAIM_COUNT
        && required_terms
            .iter()
            .all(|term| graph_claim_term_matches(text, term))
}

fn graph_claim_term_matches(text: &str, term: &str) -> bool {
    super::super::super::claim::claim_text_match_count(text, &[term.to_string()]) > 0
        || super::fact::relation_claim_matches_text(term, text)
}

fn is_graph_query_scaffold(term: &str) -> bool {
    matches!(
        term.trim().to_lowercase().as_str(),
        "answer" | "value" | "response" | "person" | "follows" | "trails"
    )
}

fn bounded_supported_seed_ids(
    ordered_seed_ids: &[i64],
    supported_seed_ids: &HashSet<i64>,
) -> Vec<i64> {
    ordered_seed_ids
        .iter()
        .copied()
        .filter(|id| supported_seed_ids.contains(id))
        .take(MAX_GRAPH_SEEDS)
        .collect()
}

fn channel_ids(channels: &[NamedChannel], name: &str) -> HashSet<i64> {
    channels
        .iter()
        .filter(|channel| channel.name == name && channel.has_hits())
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .collect()
}

fn ordered_channel_ids(channels: &[NamedChannel], name: &str) -> Vec<i64> {
    let mut seen = HashSet::new();
    channels
        .iter()
        .filter(|channel| channel.name == name && channel.has_hits())
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .filter(|id| seen.insert(*id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::graph_contract::{
        insert_graph_edge, GraphEdgeInput, GraphEdgeProvenance, GraphEdgeType, GraphNodeRef,
    };
    use rusqlite::params;

    fn insert_graph_provenance(conn: &Connection, now: i64) -> Result<(i64, i64, i64)> {
        let host_id: i64 =
            conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
                row.get(0)
            })?;
        conn.execute(
            "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch,
                                    updated_at_epoch)
             VALUES ('/repo', 'origin', 'main', ?1, ?1)",
            [now],
        )?;
        let workspace_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch,
                                  updated_at_epoch)
             VALUES (?1, '/repo', 'repo', ?2, ?2)",
            params![workspace_id, now],
        )?;
        let project_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sessions(host_id, workspace_id, project_id, session_id,
                                  started_at_epoch, last_seen_at_epoch, status)
             VALUES (?1, ?2, ?3, 'graph-claim-test', ?4, ?4, 'active')",
            params![host_id, workspace_id, project_id, now],
        )?;
        let session_row_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id,
                                         session_id, event_id, event_type, content_hash,
                                         retention_class, created_at_epoch, inserted_at_epoch)
             VALUES (?1, ?2, ?3, ?4, 'graph-claim-test', 'graph-claim-event', 'message',
                     'graph-claim-hash', 'default', ?5, ?5)",
            params![host_id, workspace_id, project_id, session_row_id, now],
        )?;
        let event_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                                           evidence_event_ids, confidence, risk_class,
                                           review_status, created_at_epoch, updated_at_epoch)
             VALUES (?1, 'project', 'decision', 'graph-claim', 'graph claim fixture',
                     ?2, 0.9, 'low', 'accepted', ?3, ?3)",
            params![project_id, format!("[{event_id}]"), now],
        )?;
        let candidate_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                                             source_candidate_id, confidence, reason,
                                             created_at_epoch)
             VALUES ('add', 'graph-claim-test', 'test', 'memory_candidate',
                     ?1, 0.9, 'graph claim fixture', ?2)",
            params![candidate_id, now],
        )?;
        Ok((event_id, candidate_id, conn.last_insert_rowid()))
    }

    fn graph_claim_plan(a: i64, b: i64, c: i64) -> QuerySearchPlan {
        QuerySearchPlan {
            expanded_terms: vec![],
            core_terms: vec![],
            claim_terms: vec!["arachne".to_string(), "signal".to_string()],
            explicit_entity_terms: vec![],
            explicit_entity_memory_ids: vec![],
            explicit_entity_neighbor_ids: vec![],
            fact_supported_memory_ids: vec![],
            graph_claim_supported_memory_ids: vec![],
            memory_type: None,
            branch: None,
            include_stale: false,
            fts_query: None,
            temporal_range: None,
            temporal_field: None,
            include_suppressed: false,
            fetch_limit: 10,
            weights: crate::retrieval::search::SearchWeights::default(),
            channels: vec![
                NamedChannel::enabled("fts", 1.0, vec![a, b]),
                NamedChannel::enabled("graph_traversal", 1.0, vec![c]),
            ],
            timings: vec![],
        }
    }

    fn explicit_entity_plan(
        explicit_entity_terms: Vec<String>,
        explicit_entity_memory_ids: Vec<i64>,
        visible_ids: &[i64],
    ) -> QuerySearchPlan {
        QuerySearchPlan {
            expanded_terms: vec![],
            core_terms: vec![],
            claim_terms: vec!["related".to_string()],
            explicit_entity_terms,
            explicit_entity_memory_ids,
            explicit_entity_neighbor_ids: vec![],
            fact_supported_memory_ids: vec![],
            graph_claim_supported_memory_ids: vec![],
            memory_type: None,
            branch: None,
            include_stale: false,
            fts_query: None,
            temporal_range: None,
            temporal_field: None,
            include_suppressed: false,
            fetch_limit: 10,
            weights: crate::retrieval::search::SearchWeights::default(),
            channels: vec![NamedChannel::enabled("entity", 1.0, visible_ids.to_vec())],
            timings: vec![],
        }
    }

    #[test]
    fn distinctive_entities_on_separate_nodes_require_a_trusted_graph_path() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let now = chrono::Utc::now().timestamp();
        let nebula = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "NebulaLatch",
            "NebulaLatch deployment record.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let harbor = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "HarborMint",
            "HarborMint successor record.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let prism = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "PrismRelay",
            "PrismRelay isolated record.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let quartz = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "QuartzGate",
            "QuartzGate isolated record.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        for (memory_id, entity) in [
            (nebula, "NebulaLatch"),
            (harbor, "HarborMint"),
            (prism, "PrismRelay"),
            (quartz, "QuartzGate"),
        ] {
            crate::retrieval::entity::link_entities(&conn, memory_id, &[entity.to_string()])?;
        }
        let (event_id, candidate_id, operation_id) = insert_graph_provenance(&conn, now)?;
        insert_graph_edge(
            &conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Supersedes,
                from_node: GraphNodeRef::memory(harbor)?,
                to_node: GraphNodeRef::memory(nebula)?,
                provenance: GraphEdgeProvenance {
                    source_event_ids: &[event_id],
                    source_candidate_id: Some(candidate_id),
                    source_operation_id: Some(operation_id),
                    confidence: Some(0.9),
                    reason: Some("cross-entity trusted path fixture"),
                },
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;

        let connected_terms = ["NebulaLatch", "HarborMint"].map(str::to_string);
        let (selected_terms, connected_anchors) =
            crate::retrieval::search::memory::claim::select_entity_anchors(
                &conn,
                &connected_terms,
                Some("/repo"),
                None,
                None,
                10,
                false,
                false,
            )?;
        assert_eq!(selected_terms, connected_terms);
        assert_eq!(
            connected_anchors,
            vec![nebula, harbor],
            "separate per-entity anchors must survive initial selection"
        );
        let connected_memories = crate::memory::get_memories_by_ids_with_suppressed_policy(
            &conn,
            &[nebula, harbor],
            Some("/repo"),
            false,
        )?;
        let mut connected_plan =
            explicit_entity_plan(selected_terms, connected_anchors, &[nebula, harbor]);
        super::super::resolve_explicit_entity_scope(
            &conn,
            "How is NebulaLatch related to HarborMint?",
            Some("/repo"),
            &[(nebula, 1.0), (harbor, 0.9)],
            &mut connected_plan,
            &connected_memories,
        )?;
        assert_eq!(
            connected_plan.explicit_entity_memory_ids,
            vec![nebula, harbor]
        );

        let disconnected_terms = ["PrismRelay", "QuartzGate"].map(str::to_string);
        let (selected_terms, disconnected_anchors) =
            crate::retrieval::search::memory::claim::select_entity_anchors(
                &conn,
                &disconnected_terms,
                Some("/repo"),
                None,
                None,
                10,
                false,
                false,
            )?;
        assert_eq!(disconnected_anchors, vec![prism, quartz]);
        let disconnected_memories = crate::memory::get_memories_by_ids_with_suppressed_policy(
            &conn,
            &[prism, quartz],
            Some("/repo"),
            false,
        )?;
        let mut disconnected_plan =
            explicit_entity_plan(selected_terms, disconnected_anchors, &[prism, quartz]);
        super::super::resolve_explicit_entity_scope(
            &conn,
            "How is PrismRelay related to QuartzGate?",
            Some("/repo"),
            &[(prism, 1.0), (quartz, 0.9)],
            &mut disconnected_plan,
            &disconnected_memories,
        )?;
        assert!(
            disconnected_plan.explicit_entity_memory_ids.is_empty(),
            "separate entity anchors without a trusted path must fail closed"
        );

        let missing_terms = ["NebulaLatch", "MissingBeacon"].map(str::to_string);
        let (selected_terms, missing_anchors) =
            crate::retrieval::search::memory::claim::select_entity_anchors(
                &conn,
                &missing_terms,
                Some("/repo"),
                None,
                None,
                10,
                false,
                false,
            )?;
        assert!(missing_anchors.is_empty());
        let mut missing_plan = explicit_entity_plan(selected_terms, missing_anchors, &[nebula]);
        super::super::resolve_explicit_entity_scope(
            &conn,
            "How is NebulaLatch related to MissingBeacon?",
            Some("/repo"),
            &[(nebula, 1.0)],
            &mut missing_plan,
            &connected_memories[..1],
        )?;
        assert!(
            missing_plan.explicit_entity_memory_ids.is_empty(),
            "a missing explicit entity must fail closed"
        );
        Ok(())
    }

    #[test]
    fn unrelated_fts_seed_cannot_authorize_its_graph_target() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let now = chrono::Utc::now().timestamp();
        let a = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "A",
            "Arachne signal confirms the deployment.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let b = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "B",
            "Unrelated bookkeeping record.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let c = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "C",
            "Graph target reachable only from B.",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let (event_id, candidate_id, operation_id) = insert_graph_provenance(&conn, now)?;
        insert_graph_edge(
            &conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Supersedes,
                from_node: GraphNodeRef::memory(c)?,
                to_node: GraphNodeRef::memory(b)?,
                provenance: GraphEdgeProvenance {
                    source_event_ids: &[event_id],
                    source_candidate_id: Some(candidate_id),
                    source_operation_id: Some(operation_id),
                    confidence: Some(0.9),
                    reason: Some("B-to-C regression fixture"),
                },
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;

        let mut plan = graph_claim_plan(a, b, c);
        resolve_graph_claim_support(&conn, Some("/repo"), &mut plan)?;

        assert!(
            plan.graph_claim_supported_memory_ids.is_empty(),
            "claim-supported A must not authorize C through unrelated seed B"
        );
        Ok(())
    }

    #[test]
    fn graph_seed_missing_query_qualifier_is_not_claim_supported() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let now = chrono::Utc::now().timestamp();
        let seed = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "港湾服务",
            "林舟验证了港湾服务。",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let mut plan = graph_claim_plan(seed, seed, seed);
        plan.claim_terms = vec![
            "验证".to_string(),
            "港湾".to_string(),
            "服务".to_string(),
            "欧洲生产环境".to_string(),
        ];

        let supported = claim_supported_seed_ids(&conn, Some("/repo"), &plan, &[seed])?;

        assert!(
            supported.is_empty(),
            "a graph seed missing the environment qualifier must not authorize traversal"
        );
        Ok(())
    }

    #[test]
    fn graph_seed_with_every_query_qualifier_is_claim_supported() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let now = chrono::Utc::now().timestamp();
        let seed = crate::memory::insert_memory_full(
            &conn,
            None,
            "/repo",
            None,
            "港湾服务",
            "林舟验证了港湾服务欧洲生产环境。",
            "decision",
            None,
            None,
            "project",
            Some(now),
        )?;
        let mut plan = graph_claim_plan(seed, seed, seed);
        plan.claim_terms = vec![
            "验证".to_string(),
            "港湾".to_string(),
            "服务".to_string(),
            "欧洲生产环境".to_string(),
        ];

        let supported = claim_supported_seed_ids(&conn, Some("/repo"), &plan, &[seed])?;

        assert_eq!(supported, vec![seed]);
        Ok(())
    }

    #[test]
    fn graph_relation_matching_accepts_aliases_without_substring_false_positives() {
        assert!(graph_claim_term_matches(
            "A reviewer signs the decision.",
            "signed"
        ));
        assert!(graph_claim_term_matches(
            "A reviewer verified the decision.",
            "signed"
        ));
        assert!(!graph_claim_term_matches(
            "The team designs workflows and assigns reviewers.",
            "signed"
        ));
    }

    #[test]
    fn graph_seed_scaffolding_does_not_weaken_required_content() {
        let terms = vec![
            "answer".to_string(),
            "follows".to_string(),
            "signal".to_string(),
            "arachne".to_string(),
        ];

        assert!(graph_seed_supports_claim(
            "Signal arachne resolves through a file path.",
            &terms
        ));
        assert!(!graph_seed_supports_claim(
            "Signal resolves through a file path.",
            &terms
        ));
    }

    #[test]
    fn supported_graph_seeds_preserve_stable_rank_order_and_cap() {
        let ordered = (1..=40).rev().collect::<Vec<_>>();
        let supported = ordered.iter().copied().collect::<HashSet<_>>();

        let selected = bounded_supported_seed_ids(&ordered, &supported);

        assert_eq!(selected, ordered[..MAX_GRAPH_SEEDS]);
    }
}
