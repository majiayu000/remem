use crate::retrieval::search::common::WeightedRankedHit;

pub(super) fn fts_ranked_hits(hits: &[(i64, f64)]) -> Vec<WeightedRankedHit> {
    let best = hits
        .iter()
        .map(|(_, score)| *score)
        .fold(f64::INFINITY, f64::min);
    let worst = hits
        .iter()
        .map(|(_, score)| *score)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = worst - best;
    hits.iter()
        .map(|(id, score)| {
            if spread.abs() < f64::EPSILON {
                WeightedRankedHit::rank_only(*id)
            } else {
                WeightedRankedHit::scored(*id, ((worst - *score) / spread).clamp(0.0, 1.0))
            }
        })
        .collect()
}

pub(super) fn rank_ordered_hits(ids: Vec<i64>) -> Vec<WeightedRankedHit> {
    ids.into_iter().map(WeightedRankedHit::rank_only).collect()
}
