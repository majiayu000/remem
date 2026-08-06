//! Usage-aware reranking for the injection path (GH947).
//!
//! `retrieval::search` reranks its retrieved candidates by access recency and
//! frequency whenever `SearchWeights::usage` is positive. The injection path
//! fused only its retrieval channels and ignored `weights.usage` entirely, so
//! raising the weight changed search results while leaving SessionStart
//! injection ordering untouched. This module closes that gap by applying the
//! same reranker to the injection channel set.

use anyhow::Result;
use rusqlite::Connection;

use crate::retrieval::search::common::distinct_hit_ids;
use crate::retrieval::search::usage_hits_for_retrieved_candidates;
use crate::retrieval::search::SearchWeights;

use super::ContextChannel;

/// Appends a usage channel over the candidates the retrieval channels already
/// surfaced. Usage never introduces a memory on its own: an id absent from
/// every retrieval channel cannot be reranked into the result.
pub(super) fn push_usage_channel(
    conn: &Connection,
    channels: &mut Vec<ContextChannel>,
    weights: SearchWeights,
) -> Result<()> {
    if weights.usage <= 0.0 {
        return Ok(());
    }
    let candidate_ids = distinct_hit_ids(channels.iter().flat_map(|channel| channel.hits.iter()));
    let hits = usage_hits_for_retrieved_candidates(conn, &candidate_ids, weights)?;
    if !hits.is_empty() {
        channels.push(ContextChannel {
            weight: weights.usage,
            hits,
        });
    }
    Ok(())
}
