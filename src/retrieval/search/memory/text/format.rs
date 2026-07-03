use crate::retrieval::memory_search::FtsMemoryHit;
use crate::retrieval::search::common::{rank_normalized_score, WeightedRankedHit};

pub(super) fn fts_normalized_hits(hits: &[FtsMemoryHit]) -> Vec<WeightedRankedHit> {
    let best = hits
        .iter()
        .map(|hit| hit.score)
        .fold(f64::INFINITY, f64::min);
    let worst = hits
        .iter()
        .map(|hit| hit.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = worst - best;
    hits.iter()
        .enumerate()
        .map(|(rank, hit)| WeightedRankedHit {
            id: hit.memory.id,
            normalized_score: if spread.abs() < f64::EPSILON {
                rank_normalized_score(rank)
            } else {
                ((worst - hit.score) / spread).clamp(0.0, 1.0)
            },
        })
        .collect()
}
