use std::collections::{BTreeMap, HashSet};

use anyhow::{bail, Result};
use rusqlite::Connection;

use super::query::{
    bridge_edges, graph_table_state, memory_is_eligible, seed_edges, EdgeRow, GraphTableState,
    MemoryEligibility,
};
use super::types::{
    GraphPathKind, GraphTraversalDiagnostics, GraphTraversalHit, GraphTraversalOutcome,
    GraphTraversalRequest, GraphTraversalStatus,
};

pub fn traverse_trusted_graph(
    conn: &Connection,
    request: GraphTraversalRequest<'_>,
) -> Result<GraphTraversalOutcome> {
    request.limits.validate()?;
    if request.seed_memory_ids.is_empty() {
        return Ok(GraphTraversalOutcome::empty(GraphTraversalStatus::NoSeed));
    }
    if request.seed_memory_ids.len() > request.limits.max_seeds {
        bail!(
            "graph seed cap exceeded: {} > {}",
            request.seed_memory_ids.len(),
            request.limits.max_seeds
        );
    }
    match graph_table_state(conn)? {
        GraphTableState::Missing => {
            return Ok(GraphTraversalOutcome::empty(
                GraphTraversalStatus::MissingTable,
            ));
        }
        GraphTableState::Empty => {
            return Ok(GraphTraversalOutcome::empty(
                GraphTraversalStatus::EmptyGraph,
            ));
        }
        GraphTableState::Populated => {}
    }

    let seed_set: HashSet<i64> = request.seed_memory_ids.iter().copied().collect();
    let mut hits = BTreeMap::<i64, GraphTraversalHit>::new();
    let mut diagnostics = GraphTraversalDiagnostics::default();
    for (seed_rank, seed_id) in request.seed_memory_ids.iter().copied().enumerate() {
        let edges = seed_edges(
            conn,
            seed_id,
            request.reference_time_epoch,
            request.limits.max_degree_per_node,
        )?;
        ensure_degree_cap(&edges, request.limits.max_degree_per_node, seed_id)?;
        for edge in edges {
            record_edge_scan(&mut diagnostics, request.limits.max_edges_scanned)?;
            if edge.edge_trust == "diagnostic_hint" {
                diagnostics.diagnostic_hint_edges += 1;
                continue;
            }
            if edge.edge_trust != "trusted" {
                bail!(
                    "graph edge {} has unknown trust {}",
                    edge.id,
                    edge.edge_trust
                );
            }
            match edge.edge_type.as_str() {
                "supersedes" => {
                    if edge.to_node_kind == "memory"
                        && edge.to_node_id == seed_id
                        && edge.from_node_kind == "memory"
                    {
                        consider_hit(
                            conn,
                            &request,
                            &seed_set,
                            &mut hits,
                            &mut diagnostics,
                            GraphTraversalHit {
                                memory_id: edge.from_node_id,
                                hop_count: 1,
                                path_kind: GraphPathKind::Supersedes,
                                min_confidence: edge.confidence,
                                seed_rank,
                            },
                        )?;
                    }
                }
                "mentions" | "touches_file"
                    if edge.from_node_kind == "memory" && edge.from_node_id == seed_id =>
                {
                    let expected_kind = if edge.edge_type == "mentions" {
                        "entity"
                    } else {
                        "file"
                    };
                    if edge.to_node_kind != expected_kind {
                        bail!(
                            "graph edge {} has invalid {} bridge kind {}",
                            edge.id,
                            edge.edge_type,
                            edge.to_node_kind
                        );
                    }
                    expand_bridge(
                        conn,
                        &request,
                        &seed_set,
                        &mut hits,
                        &mut diagnostics,
                        seed_rank,
                        &edge,
                    )?;
                }
                "extracted_from" => diagnostics.extracted_from_edges += 1,
                _ => diagnostics.ignored_trusted_edges += 1,
            }
        }
    }

    let mut hits = hits.into_values().collect::<Vec<_>>();
    hits.sort_by(compare_hits);
    Ok(GraphTraversalOutcome {
        status: if hits.is_empty() {
            GraphTraversalStatus::NoExpansion
        } else {
            GraphTraversalStatus::Ready
        },
        hits,
        diagnostics,
    })
}

fn expand_bridge(
    conn: &Connection,
    request: &GraphTraversalRequest<'_>,
    seed_set: &HashSet<i64>,
    hits: &mut BTreeMap<i64, GraphTraversalHit>,
    diagnostics: &mut GraphTraversalDiagnostics,
    seed_rank: usize,
    first: &EdgeRow,
) -> Result<()> {
    let edges = bridge_edges(
        conn,
        &first.edge_type,
        &first.to_node_kind,
        first.to_node_id,
        request.reference_time_epoch,
        request.limits.max_degree_per_node,
    )?;
    ensure_degree_cap(&edges, request.limits.max_degree_per_node, first.to_node_id)?;
    let path_kind = if first.edge_type == "mentions" {
        GraphPathKind::Mentions
    } else {
        GraphPathKind::TouchesFile
    };
    for second in edges {
        record_edge_scan(diagnostics, request.limits.max_edges_scanned)?;
        consider_hit(
            conn,
            request,
            seed_set,
            hits,
            diagnostics,
            GraphTraversalHit {
                memory_id: second.from_node_id,
                hop_count: 2,
                path_kind,
                min_confidence: first.confidence.min(second.confidence),
                seed_rank,
            },
        )?;
    }
    Ok(())
}

fn consider_hit(
    conn: &Connection,
    request: &GraphTraversalRequest<'_>,
    seed_set: &HashSet<i64>,
    hits: &mut BTreeMap<i64, GraphTraversalHit>,
    diagnostics: &mut GraphTraversalDiagnostics,
    hit: GraphTraversalHit,
) -> Result<()> {
    if seed_set.contains(&hit.memory_id) {
        return Ok(());
    }
    diagnostics.candidates_considered += 1;
    let eligible = memory_is_eligible(
        conn,
        hit.memory_id,
        MemoryEligibility {
            project: request.project,
            memory_type: request.memory_type,
            branch: request.branch,
            include_inactive: request.include_inactive,
        },
    )?;
    if !eligible {
        diagnostics.targets_filtered += 1;
        return Ok(());
    }
    if let Some(existing) = hits.get(&hit.memory_id) {
        if compare_hits(&hit, existing).is_ge() {
            return Ok(());
        }
    }
    hits.insert(hit.memory_id, hit);
    if hits.len() > request.limits.max_candidates {
        bail!(
            "graph candidate cap exceeded: {} > {}",
            hits.len(),
            request.limits.max_candidates
        );
    }
    Ok(())
}

fn ensure_degree_cap(edges: &[EdgeRow], cap: usize, node_id: i64) -> Result<()> {
    if edges.len() > cap {
        bail!("graph degree cap exceeded for node {node_id}: > {cap}");
    }
    Ok(())
}

fn record_edge_scan(
    diagnostics: &mut GraphTraversalDiagnostics,
    max_edges_scanned: usize,
) -> Result<()> {
    diagnostics.edges_scanned += 1;
    if diagnostics.edges_scanned > max_edges_scanned {
        bail!(
            "graph edge scan cap exceeded: {} > {}",
            diagnostics.edges_scanned,
            max_edges_scanned
        );
    }
    Ok(())
}

fn compare_hits(left: &GraphTraversalHit, right: &GraphTraversalHit) -> std::cmp::Ordering {
    left.hop_count
        .cmp(&right.hop_count)
        .then_with(|| left.path_kind.cmp(&right.path_kind))
        .then_with(|| right.min_confidence.total_cmp(&left.min_confidence))
        .then_with(|| left.seed_rank.cmp(&right.seed_rank))
        .then_with(|| left.memory_id.cmp(&right.memory_id))
}
