use super::*;

#[test]
fn test_ndcg_perfect_ranking() {
    let relevance = vec![3.0, 2.0, 1.0, 0.0];
    let score = ndcg_at_k(&relevance, 4);
    assert!((score - 1.0).abs() < 0.001, "perfect ranking should be 1.0");
}

#[test]
fn test_ndcg_worst_ranking() {
    let relevance = vec![0.0, 0.0, 0.0, 3.0];
    let score = ndcg_at_k(&relevance, 4);
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
