use std::collections::HashMap;

use crate::memory::Memory;

pub(crate) fn sanitize_fts_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .map(|token| {
            let escaped = token.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect();
    if tokens.len() <= 1 {
        tokens.join("")
    } else {
        tokens.join(" OR ")
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WeightedRankedHit {
    pub id: i64,
    pub normalized_score: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WeightedRankedChannel<'a> {
    pub weight: f64,
    pub hits: &'a [WeightedRankedHit],
}

pub(crate) fn weighted_ranked_fuse(
    channels: &[WeightedRankedChannel<'_>],
    k: f64,
) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for channel in channels {
        if channel.weight <= 0.0 {
            continue;
        }
        for (rank, hit) in channel.hits.iter().enumerate() {
            *scores.entry(hit.id).or_default() +=
                weighted_rank_score(channel.weight, k, rank, hit.normalized_score);
        }
    }
    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    results
}

pub(crate) fn weighted_rank_score(weight: f64, k: f64, rank: usize, normalized_score: f64) -> f64 {
    let rank_score = 1.0 / (k + rank as f64 + 1.0);
    weight * rank_score * (1.0 + normalized_score.clamp(0.0, 1.0))
}

pub(crate) fn rank_normalized_score(rank: usize) -> f64 {
    1.0 / (rank as f64 + 1.0)
}

pub(super) fn paginate_memories(memories: Vec<Memory>, limit: i64, offset: i64) -> Vec<Memory> {
    let start = offset.max(0) as usize;
    if start >= memories.len() {
        return vec![];
    }
    let end = (start + limit.max(0) as usize).min(memories.len());
    memories[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_ranked_fusion_prefers_one_strong_channel_over_many_weak_hits() {
        let strong = [WeightedRankedHit {
            id: 1,
            normalized_score: 1.0,
        }];
        let weak_a = [WeightedRankedHit {
            id: 2,
            normalized_score: 0.0,
        }];
        let weak_b = [WeightedRankedHit {
            id: 2,
            normalized_score: 0.0,
        }];
        let weak_c = [WeightedRankedHit {
            id: 2,
            normalized_score: 0.0,
        }];

        let fused = weighted_ranked_fuse(
            &[
                WeightedRankedChannel {
                    weight: 3.0,
                    hits: &strong,
                },
                WeightedRankedChannel {
                    weight: 1.0,
                    hits: &weak_a,
                },
                WeightedRankedChannel {
                    weight: 1.0,
                    hits: &weak_b,
                },
                WeightedRankedChannel {
                    weight: 1.0,
                    hits: &weak_c,
                },
            ],
            60.0,
        );

        assert_eq!(fused.first().map(|(id, _)| *id), Some(1));
    }

    #[test]
    fn weighted_ranked_fusion_breaks_equal_scores_by_memory_id() {
        let a = [WeightedRankedHit {
            id: 20,
            normalized_score: 0.0,
        }];
        let b = [WeightedRankedHit {
            id: 10,
            normalized_score: 0.0,
        }];

        let fused = weighted_ranked_fuse(
            &[
                WeightedRankedChannel {
                    weight: 1.0,
                    hits: &a,
                },
                WeightedRankedChannel {
                    weight: 1.0,
                    hits: &b,
                },
            ],
            60.0,
        );

        assert_eq!(
            fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![10, 20]
        );
    }
}
