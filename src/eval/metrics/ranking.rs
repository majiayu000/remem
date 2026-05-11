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
        .map(|(index, &rel)| rel / (index as f64 + 2.0).log2())
        .sum();

    let mut ideal = relevance.to_vec();
    ideal.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let idcg: f64 = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(index, &rel)| rel / (index as f64 + 2.0).log2())
        .sum();

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}
