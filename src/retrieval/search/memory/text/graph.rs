use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::perf::time_result;
use crate::retrieval::search::common::{rank_normalized_score, WeightedRankedHit};

use super::super::{suppression_filter, SearchWeights};
use super::NamedChannel;

#[allow(clippy::too_many_arguments)]
pub(super) fn append_graph_channel(
    conn: &Connection,
    channels: &mut Vec<NamedChannel>,
    timings: &mut Vec<crate::perf::PhaseTiming>,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    include_stale: bool,
    include_suppressed: bool,
    fetch_limit: i64,
    weights: SearchWeights,
) -> Result<()> {
    if weights.graph <= 0.0 {
        channels.push(NamedChannel::disabled(
            "graph_traversal",
            weights.graph,
            "graph channel weight is zero",
        ));
        return Ok(());
    }

    let seed_ids = seed_ids(channels, 32);
    let outcome = time_result(timings, "graph_traversal", || {
        crate::retrieval::graph::traverse_trusted_graph(
            conn,
            crate::retrieval::graph::GraphTraversalRequest {
                seed_memory_ids: &seed_ids,
                project,
                memory_type,
                branch,
                include_inactive: include_stale,
                reference_time_epoch: chrono::Utc::now().timestamp(),
                limits: crate::retrieval::graph::GraphTraversalLimits::for_search(fetch_limit),
            },
        )
    })?;
    if let Some(reason) = outcome.status.disabled_reason() {
        channels.push(
            NamedChannel::disabled("graph_traversal", weights.graph, reason)
                .with_candidates_scanned(outcome.diagnostics.edges_scanned),
        );
        return Ok(());
    }

    let hits = outcome
        .hits
        .into_iter()
        .enumerate()
        .map(|(rank, hit)| WeightedRankedHit {
            id: hit.memory_id,
            normalized_score: rank_normalized_score(rank),
        })
        .collect::<Vec<_>>();
    let hits = suppression_filter::weighted_hits(conn, hits, include_suppressed)?;
    channels.push(
        NamedChannel::enabled_with_hits("graph_traversal", weights.graph, hits)
            .with_candidates_scanned(outcome.diagnostics.edges_scanned),
    );
    Ok(())
}

fn seed_ids(channels: &[NamedChannel], max_seeds: usize) -> Vec<i64> {
    let mut seen = HashSet::new();
    channels
        .iter()
        .filter(|channel| channel.name == "fts" || channel.name == "vector")
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .filter(|id| seen.insert(*id))
        .take(max_seeds)
        .collect()
}
