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
    /// A calibrated channel-strength signal in `[0, 1]`.
    ///
    /// `None` means the channel only provides an ordering. Its contribution
    /// remains pure weighted RRF instead of counting rank a second time as a
    /// synthetic score.
    pub normalized_score: Option<f64>,
}

impl WeightedRankedHit {
    pub(crate) const fn rank_only(id: i64) -> Self {
        Self {
            id,
            normalized_score: None,
        }
    }

    pub(crate) const fn scored(id: i64, normalized_score: f64) -> Self {
        Self {
            id,
            normalized_score: Some(normalized_score),
        }
    }
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

pub(crate) fn reciprocal_rank_score(k: f64, rank: usize) -> f64 {
    1.0 / (k + rank as f64 + 1.0)
}

pub(crate) fn weighted_rank_score(
    weight: f64,
    k: f64,
    rank: usize,
    normalized_score: Option<f64>,
) -> f64 {
    let signal_boost = normalized_score
        .map(|score| score.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    weight * reciprocal_rank_score(k, rank) * (1.0 + signal_boost)
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
        let strong = [WeightedRankedHit::scored(1, 1.0)];
        let weak_a = [WeightedRankedHit::rank_only(2)];
        let weak_b = [WeightedRankedHit::rank_only(2)];
        let weak_c = [WeightedRankedHit::rank_only(2)];

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
        let a = [WeightedRankedHit::rank_only(20)];
        let b = [WeightedRankedHit::rank_only(10)];

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

    #[test]
    fn rank_only_channels_do_not_count_rank_twice() {
        let primary = [
            WeightedRankedHit::rank_only(1),
            WeightedRankedHit::rank_only(2),
        ];
        let corroborating = [WeightedRankedHit::rank_only(2)];

        let fused = weighted_ranked_fuse(
            &[
                WeightedRankedChannel {
                    weight: 1.0,
                    hits: &primary,
                },
                WeightedRankedChannel {
                    weight: 0.1,
                    hits: &corroborating,
                },
            ],
            60.0,
        );

        assert_eq!(
            fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![2, 1],
            "independent corroboration must beat a synthetic second rank curve"
        );
        assert_eq!(
            weighted_rank_score(1.0, 60.0, 1, None),
            reciprocal_rank_score(60.0, 1)
        );
    }
}
