use std::collections::HashMap;

const RRF_K: f64 = 60.0;
const SECOND_HOP_WEIGHT: f64 = 0.5;

pub(crate) fn rank_merged_ids(first_hop_ids: &[i64], second_hop_ids: &[i64], limit: i64) -> Vec<i64> {
    if limit <= 0 {
        return vec![];
    }

    let mut scores: HashMap<i64, f64> = HashMap::new();

    for (rank, id) in first_hop_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }

    for (rank, id) in second_hop_ids.iter().enumerate() {
        *scores.entry(*id).or_default() += SECOND_HOP_WEIGHT / (RRF_K + rank as f64 + 1.0);
    }

    let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .take(limit as usize)
        .map(|(id, _)| id)
        .collect()
}
