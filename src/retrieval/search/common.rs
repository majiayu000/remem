use std::collections::HashMap;

use anyhow::{bail, ensure, Result};

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

/// Distinct memory ids across already-retrieved hits, ascending.
///
/// Shared by the `retrieval::search` plan and the injection path so both feed
/// the usage reranker the same candidate set. Order is not significant: the
/// ids are consumed as a `WHERE id IN (...)` set.
pub(crate) fn distinct_hit_ids<'a>(hits: impl Iterator<Item = &'a WeightedRankedHit>) -> Vec<i64> {
    let mut ids = hits.map(|hit| hit.id).collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub(crate) fn weighted_ranked_fuse(
    channels: &[WeightedRankedChannel<'_>],
    k: f64,
) -> Result<Vec<(i64, f64)>> {
    validate_rrf_k(k)?;
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for channel in channels {
        if !channel.weight.is_finite() {
            bail!("RRF channel weight must be finite, got {}", channel.weight);
        }
        if channel.weight <= 0.0 {
            continue;
        }
        for (rank, hit) in channel.hits.iter().enumerate() {
            let contribution = weighted_rank_score(channel.weight, k, rank, hit.normalized_score)?;
            let score = scores.entry(hit.id).or_default();
            let total = *score + contribution;
            ensure!(
                total.is_finite(),
                "RRF score overflow for memory {}",
                hit.id
            );
            *score = total;
        }
    }
    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(results)
}

fn validate_rrf_k(k: f64) -> Result<()> {
    ensure!(k.is_finite(), "RRF k must be finite, got {k}");
    ensure!(k >= 0.0, "RRF k must be non-negative, got {k}");
    Ok(())
}

pub(crate) fn reciprocal_rank_score(k: f64, rank: usize) -> Result<f64> {
    validate_rrf_k(k)?;
    let denominator = k + rank as f64 + 1.0;
    ensure!(
        denominator.is_finite(),
        "RRF denominator overflow for k={k} rank={rank}"
    );
    Ok(1.0 / denominator)
}

pub(crate) fn calibrated_signal(normalized_score: Option<f64>) -> Result<Option<f64>> {
    normalized_score
        .map(|score| {
            ensure!(
                score.is_finite(),
                "RRF normalized signal must be finite, got {score}"
            );
            Ok(score.clamp(0.0, 1.0))
        })
        .transpose()
}

pub(crate) fn calibrated_vector_similarity(distance: f32, max_vector_distance: f32) -> Result<f64> {
    ensure!(
        distance.is_finite(),
        "vector distance must be finite, got {distance}"
    );
    let threshold = f64::from(max_vector_distance);
    ensure!(
        threshold.is_finite() && threshold > 0.0,
        "max vector distance must be finite and positive, got {max_vector_distance}"
    );
    let score = (threshold - f64::from(distance)) / threshold;
    ensure!(
        score.is_finite(),
        "vector similarity must be finite for distance={distance} threshold={max_vector_distance}"
    );
    Ok(score.clamp(0.0, 1.0))
}

pub(crate) fn calibrated_vector_hits(
    hits: impl IntoIterator<Item = (i64, f32)>,
    max_vector_distance: f32,
) -> Result<Vec<WeightedRankedHit>> {
    ensure!(
        max_vector_distance.is_finite(),
        "max vector distance must be finite, got {max_vector_distance}"
    );
    let mut ranked = Vec::new();
    for (id, distance) in hits {
        ensure!(
            distance.is_finite(),
            "vector distance must be finite, got {distance}"
        );
        if max_vector_distance <= 0.0 || distance > max_vector_distance {
            continue;
        }
        ranked.push(WeightedRankedHit::scored(
            id,
            calibrated_vector_similarity(distance, max_vector_distance)?,
        ));
    }
    Ok(ranked)
}

pub(crate) fn weighted_rank_score(
    weight: f64,
    k: f64,
    rank: usize,
    normalized_score: Option<f64>,
) -> Result<f64> {
    ensure!(
        weight.is_finite(),
        "RRF weight must be finite, got {weight}"
    );
    if weight <= 0.0 {
        return Ok(0.0);
    }
    let signal_boost = calibrated_signal(normalized_score)?.unwrap_or(0.0);
    let score = weight * reciprocal_rank_score(k, rank)? * (1.0 + signal_boost);
    ensure!(
        score.is_finite(),
        "RRF weighted score overflow for weight={weight} k={k} rank={rank}"
    );
    Ok(score)
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
    fn weighted_ranked_fusion_prefers_one_strong_channel_over_many_weak_hits() -> Result<()> {
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
        )?;

        assert_eq!(fused.first().map(|(id, _)| *id), Some(1));
        Ok(())
    }

    #[test]
    fn weighted_ranked_fusion_breaks_equal_scores_by_memory_id() -> Result<()> {
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
        )?;

        assert_eq!(
            fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![10, 20]
        );
        Ok(())
    }

    #[test]
    fn rank_only_channels_do_not_count_rank_twice() -> Result<()> {
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
        )?;

        assert_eq!(
            fused.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![2, 1],
            "independent corroboration must beat a synthetic second rank curve"
        );
        assert_eq!(
            weighted_rank_score(1.0, 60.0, 1, None)?,
            reciprocal_rank_score(60.0, 1)?
        );
        Ok(())
    }

    #[test]
    fn weighted_ranked_fusion_rejects_non_finite_inputs() {
        let ranked = [WeightedRankedHit::rank_only(1)];
        let scored = [WeightedRankedHit::scored(1, f64::NAN)];

        let weight_error = weighted_ranked_fuse(
            &[WeightedRankedChannel {
                weight: f64::NAN,
                hits: &ranked,
            }],
            60.0,
        )
        .expect_err("NaN channel weight must fail");
        assert!(weight_error.to_string().contains("weight must be finite"));

        let signal_error = weighted_ranked_fuse(
            &[WeightedRankedChannel {
                weight: 1.0,
                hits: &scored,
            }],
            60.0,
        )
        .expect_err("NaN signal must fail");
        assert!(signal_error.to_string().contains("signal must be finite"));

        let k_error = weighted_ranked_fuse(
            &[WeightedRankedChannel {
                weight: 1.0,
                hits: &ranked,
            }],
            f64::INFINITY,
        )
        .expect_err("infinite k must fail");
        assert!(k_error.to_string().contains("k must be finite"));
    }

    #[test]
    fn weighted_ranked_fusion_rejects_score_overflow() {
        let scored = [WeightedRankedHit::scored(1, 1.0)];
        let error = weighted_ranked_fuse(
            &[WeightedRankedChannel {
                weight: f64::MAX,
                hits: &scored,
            }],
            0.0,
        )
        .expect_err("overflowing score must fail");

        assert!(error.to_string().contains("score overflow"));
    }

    #[test]
    fn vector_similarity_rejects_zero_threshold() {
        let error = calibrated_vector_similarity(0.0, 0.0)
            .expect_err("zero threshold must not produce a NaN score");

        assert!(error.to_string().contains("finite and positive"));
    }

    #[test]
    fn vector_hit_filter_rejects_non_finite_inputs_before_filtering() {
        let threshold_error = calibrated_vector_hits([], f32::NAN)
            .expect_err("a NaN threshold must fail even when there are no hits");
        assert!(threshold_error
            .to_string()
            .contains("max vector distance must be finite"));

        let distance_error = calibrated_vector_hits([(1, f32::NAN)], 1.0)
            .expect_err("a NaN distance must not be silently filtered");
        assert!(distance_error
            .to_string()
            .contains("vector distance must be finite"));

        assert!(calibrated_vector_hits([(1, 0.25)], -1.0)
            .expect("a finite closed threshold remains a disabled channel")
            .is_empty());
    }
}
