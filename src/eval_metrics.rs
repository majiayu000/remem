/// NDCG@k (Normalized Discounted Cumulative Gain).
/// Measures ranking quality: are relevant results near the top?
/// `relevance` is an ordered list of relevance grades for the returned results.
pub fn ndcg_at_k(relevance: &[f64], k: usize) -> f64 {
    if relevance.is_empty() || k == 0 {
        return 0.0;
    }
    let k = k.min(relevance.len());

    let dcg: f64 = relevance
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| rel / (i as f64 + 2.0).log2())
        .sum();

    let mut ideal = relevance.to_vec();
    ideal.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let idcg: f64 = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| rel / (i as f64 + 2.0).log2())
        .sum();

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

/// MRR (Mean Reciprocal Rank).
/// Position of the first relevant result. Returns 1/rank.
pub fn reciprocal_rank(result_ids: &[i64], relevant_ids: &[i64]) -> f64 {
    for (i, id) in result_ids.iter().enumerate() {
        if relevant_ids.contains(id) {
            return 1.0 / (i as f64 + 1.0);
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
        return 1.0; // no relevant items expected = perfect recall
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ndcg_perfect_ranking() {
        // Perfect ranking: [3, 2, 1, 0]
        let rel = vec![3.0, 2.0, 1.0, 0.0];
        let score = ndcg_at_k(&rel, 4);
        assert!((score - 1.0).abs() < 0.001, "perfect ranking should be 1.0");
    }

    #[test]
    fn test_ndcg_worst_ranking() {
        // Worst ranking: [0, 0, 0, 3]
        let rel = vec![0.0, 0.0, 0.0, 3.0];
        let score = ndcg_at_k(&rel, 4);
        assert!(score < 0.5, "worst ranking should be low: {}", score);
    }

    #[test]
    fn test_ndcg_empty() {
        assert_eq!(ndcg_at_k(&[], 5), 0.0);
    }

    #[test]
    fn test_reciprocal_rank_first() {
        assert_eq!(reciprocal_rank(&[10, 20, 30], &[10]), 1.0);
    }

    #[test]
    fn test_reciprocal_rank_third() {
        let rr = reciprocal_rank(&[10, 20, 30], &[30]);
        assert!((rr - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_reciprocal_rank_miss() {
        assert_eq!(reciprocal_rank(&[10, 20, 30], &[99]), 0.0);
    }

    #[test]
    fn test_precision_at_k() {
        assert_eq!(precision_at_k(&[1, 2, 3, 4, 5], &[1, 3, 5], 5), 0.6);
        assert_eq!(precision_at_k(&[1, 2, 3], &[1, 2, 3], 3), 1.0);
        assert_eq!(precision_at_k(&[1, 2, 3], &[99], 3), 0.0);
    }

    #[test]
    fn test_recall_at_k() {
        assert_eq!(recall_at_k(&[1, 2, 3], &[1, 2, 3, 4], 3), 0.75);
        assert_eq!(recall_at_k(&[1, 2], &[1, 2], 5), 1.0);
    }

    #[test]
    fn test_hit_at_k() {
        assert_eq!(hit_at_k(&[10, 20, 30], &[30], 3), 1.0);
        assert_eq!(hit_at_k(&[10, 20, 30], &[99], 3), 0.0);
    }
}
