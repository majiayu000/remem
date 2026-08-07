const RRF_K: f64 = 60.0;
const MAX_VECTOR_DISTANCE: f32 = 0.51;
const FTS_WEIGHT: f64 = 2.5;
const VECTOR_WEIGHT: f64 = 3.0;
const ENTITY_WEIGHT: f64 = 1.25;
const GRAPH_WEIGHT: f64 = 0.75;
const TEMPORAL_WEIGHT: f64 = 1.0;
const FACT_WEIGHT: f64 = 1.4;
const LIKE_FALLBACK_WEIGHT: f64 = 0.25;
/// GH-947: calibrated by `eval-weight-grid` (eval/weight-grid/report.json).
/// On the golden fixture corpus every shadowed usage weight in {0.25, 0.75,
/// 1.5} left scored-query, abstention, and top-result outcomes unchanged, so
/// the smallest shadowed weight is the gray-release default: it closes the M5
/// write-only feedback loop with the least reordering influence. Operators can
/// roll back with `REMEM_USAGE_WEIGHT=0`.
const USAGE_WEIGHT: f64 = 0.25;
const USAGE_RECENCY_HALF_LIFE_DAYS: f64 = 30.0;
const MIN_EVIDENCE_CONFIDENCE: f64 = 0.62;

/// Operator override for the usage channel weight (GH-947 gray release).
/// `REMEM_USAGE_WEIGHT=0` restores the pre-rollout ranking byte-for-byte.
pub(crate) const USAGE_WEIGHT_ENV: &str = "REMEM_USAGE_WEIGHT";

#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct SearchWeights {
    pub fts: f64,
    pub vector: f64,
    pub entity: f64,
    #[serde(default = "default_graph_weight")]
    pub graph: f64,
    pub temporal: f64,
    #[serde(default = "default_fact_weight")]
    pub fact: f64,
    pub like_fallback: f64,
    #[serde(default)]
    pub usage: f64,
    #[serde(default = "default_usage_recency_half_life_days")]
    pub usage_recency_half_life_days: f64,
    pub max_vector_distance: f32,
    pub rrf_k: f64,
    pub min_evidence_confidence: f64,
}

impl Default for SearchWeights {
    fn default() -> Self {
        Self {
            fts: FTS_WEIGHT,
            vector: VECTOR_WEIGHT,
            entity: ENTITY_WEIGHT,
            graph: GRAPH_WEIGHT,
            temporal: TEMPORAL_WEIGHT,
            fact: FACT_WEIGHT,
            like_fallback: LIKE_FALLBACK_WEIGHT,
            usage: USAGE_WEIGHT,
            usage_recency_half_life_days: USAGE_RECENCY_HALF_LIFE_DAYS,
            max_vector_distance: MAX_VECTOR_DISTANCE,
            rrf_k: RRF_K,
            min_evidence_confidence: MIN_EVIDENCE_CONFIDENCE,
        }
    }
}

impl SearchWeights {
    /// Weights for production retrieval entry points (injection + search).
    ///
    /// Returns the calibrated defaults with the GH-947 operator override
    /// applied: a finite `REMEM_USAGE_WEIGHT` value replaces the usage weight,
    /// and `0` disables the usage channel, reproducing the pre-rollout
    /// ranking exactly. An unset, empty, or unparseable value keeps the
    /// calibrated default; parse failures log at error level instead of
    /// silently degrading.
    pub(crate) fn production() -> Self {
        let mut weights = Self::default();
        let Ok(raw) = std::env::var(USAGE_WEIGHT_ENV) else {
            return weights;
        };
        let raw = raw.trim();
        if raw.is_empty() {
            return weights;
        }
        match raw.parse::<f64>() {
            Ok(value) if value.is_finite() => weights.usage = value,
            _ => crate::log::error(
                "retrieval",
                &format!(
                    "ignoring invalid {USAGE_WEIGHT_ENV}={raw:?}; using calibrated default {}",
                    weights.usage
                ),
            ),
        }
        weights
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        let channel_weights = [
            ("fts", self.fts),
            ("vector", self.vector),
            ("entity", self.entity),
            ("graph", self.graph),
            ("temporal", self.temporal),
            ("fact", self.fact),
            ("like_fallback", self.like_fallback),
            ("usage", self.usage),
        ];
        for (name, weight) in channel_weights {
            anyhow::ensure!(
                weight.is_finite(),
                "search weight {name} must be finite, got {weight}"
            );
        }
        anyhow::ensure!(
            self.usage_recency_half_life_days.is_finite(),
            "usage_recency_half_life_days must be finite, got {}",
            self.usage_recency_half_life_days
        );
        anyhow::ensure!(
            self.max_vector_distance.is_finite(),
            "max_vector_distance must be finite, got {}",
            self.max_vector_distance
        );
        anyhow::ensure!(
            self.rrf_k.is_finite() && self.rrf_k >= 0.0,
            "rrf_k must be finite and non-negative, got {}",
            self.rrf_k
        );
        anyhow::ensure!(
            self.min_evidence_confidence.is_finite(),
            "min_evidence_confidence must be finite, got {}",
            self.min_evidence_confidence
        );
        Ok(())
    }
}

fn default_fact_weight() -> f64 {
    FACT_WEIGHT
}

fn default_graph_weight() -> f64 {
    GRAPH_WEIGHT
}

fn default_usage_recency_half_life_days() -> f64 {
    USAGE_RECENCY_HALF_LIFE_DAYS
}

#[cfg(test)]
mod tests {
    use super::SearchWeights;

    #[test]
    fn production_weights_honor_usage_env_override() {
        // One test fn keeps the process-global env mutations sequential.
        // Unset: the calibrated GH-947 default.
        std::env::remove_var(super::USAGE_WEIGHT_ENV);
        let calibrated = SearchWeights::production();
        assert_eq!(calibrated.usage, super::USAGE_WEIGHT);
        assert!(calibrated.usage > 0.0);

        // Explicit zero is the rollback: identical to the pre-rollout weights,
        // and the usage channel gate (`> 0.0`) keeps every downstream score
        // byte-identical to the pre-GH-947 behavior.
        std::env::set_var(super::USAGE_WEIGHT_ENV, "0");
        let rolled_back = SearchWeights::production();
        assert_eq!(rolled_back.usage, 0.0);
        assert_eq!(
            rolled_back,
            SearchWeights {
                usage: 0.0,
                ..SearchWeights::default()
            }
        );

        // A finite override wins over the default.
        std::env::set_var(super::USAGE_WEIGHT_ENV, " 0.75 ");
        assert_eq!(SearchWeights::production().usage, 0.75);

        // Unparseable or non-finite values keep the calibrated default.
        for invalid in ["junk", "1e999", "NaN"] {
            std::env::set_var(super::USAGE_WEIGHT_ENV, invalid);
            assert_eq!(
                SearchWeights::production().usage,
                super::USAGE_WEIGHT,
                "invalid override {invalid:?} must fall back to the calibrated default"
            );
        }
        std::env::remove_var(super::USAGE_WEIGHT_ENV);
    }

    #[test]
    fn validation_rejects_every_non_finite_numeric_setting() {
        let mut invalid = Vec::new();
        macro_rules! reject_f64 {
            ($field:ident) => {
                for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    invalid.push((
                        stringify!($field),
                        SearchWeights {
                            $field: value,
                            ..SearchWeights::default()
                        },
                    ));
                }
            };
        }
        reject_f64!(fts);
        reject_f64!(vector);
        reject_f64!(entity);
        reject_f64!(graph);
        reject_f64!(temporal);
        reject_f64!(fact);
        reject_f64!(like_fallback);
        reject_f64!(usage);
        reject_f64!(usage_recency_half_life_days);
        reject_f64!(rrf_k);
        reject_f64!(min_evidence_confidence);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            invalid.push((
                "max_vector_distance",
                SearchWeights {
                    max_vector_distance: value,
                    ..SearchWeights::default()
                },
            ));
        }

        for (field, weights) in invalid {
            let error = weights.validate().expect_err(field);
            assert!(error.to_string().contains(field), "{field}: {error:#}");
        }
    }

    #[test]
    fn validation_preserves_finite_disabled_weight_semantics() {
        let weights = SearchWeights {
            fact: -1.0,
            usage: 0.0,
            ..SearchWeights::default()
        };
        weights
            .validate()
            .expect("finite disabled weights are valid");
    }
}
