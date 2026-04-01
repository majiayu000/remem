/// MRR (Mean Reciprocal Rank).
/// Position of the first relevant result. Returns 1/rank.
pub fn reciprocal_rank(result_ids: &[i64], relevant_ids: &[i64]) -> f64 {
    for (index, id) in result_ids.iter().enumerate() {
        if relevant_ids.contains(id) {
            return 1.0 / (index as f64 + 1.0);
        }
    }
    0.0
}

/// Precision@k: fraction of top-k results that are relevant.
pub fn precision_at_k(result_ids: &[i64], relevant_ids: &[i64], k: usize) -> f64 {
    if k == 0 || result_ids.is_empty() {
        return 0.0;
    }
    let k = k.min(result_ids.len());
    let hits = result_ids
        .iter()
        .take(k)
        .filter(|id| relevant_ids.contains(id))
        .count();
    hits as f64 / k as f64
}

/// Recall@k: fraction of all relevant items that appear in top-k.
pub fn recall_at_k(result_ids: &[i64], relevant_ids: &[i64], k: usize) -> f64 {
    if relevant_ids.is_empty() {
        return 1.0;
    }
    let k = k.min(result_ids.len());
    let hits = result_ids
        .iter()
        .take(k)
        .filter(|id| relevant_ids.contains(id))
        .count();
    hits as f64 / relevant_ids.len() as f64
}

/// Hit@k: 1.0 if any relevant result in top-k, else 0.0.
pub fn hit_at_k(result_ids: &[i64], relevant_ids: &[i64], k: usize) -> f64 {
    let k = k.min(result_ids.len());
    if result_ids
        .iter()
        .take(k)
        .any(|id| relevant_ids.contains(id))
    {
        1.0
    } else {
        0.0
    }
}
