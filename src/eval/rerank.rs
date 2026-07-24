//! Rerank A/B promote gate (GH-851 B-012).
//!
//! The gate consumes a paired rerank-off/rerank-on artifact produced on one
//! commit, one `eval/golden.json` hash, one database fixture, one model
//! manifest, and one machine. It never fabricates metrics: enabling rerank by
//! default requires a real artifact produced with the locally installed model
//! (pending runtime evidence; see the GH-851 implementation PR).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Minimum absolute improvement the preregistered primary metric must show
/// on the paraphrase+associative combined set (spec-frozen: `0.05`).
pub const PRIMARY_METRIC_MIN_DELTA: f64 = 0.05;

/// Closed set of preregisterable primary metrics. The metric must be frozen
/// in the artifact header before results are inspected and cannot be swapped
/// after seeing the numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RerankPrimaryMetric {
    CombinedMrrAt10,
    CombinedHitAt5,
}

impl RerankPrimaryMetric {
    fn value(self, metrics: &RerankAbMetrics) -> f64 {
        match self {
            Self::CombinedMrrAt10 => metrics.combined_mrr_at_10,
            Self::CombinedHitAt5 => metrics.combined_hit_at_5,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::CombinedMrrAt10 => "combined_mrr_at_10",
            Self::CombinedHitAt5 => "combined_hit_at_5",
        }
    }
}

/// One side (off or on) of the paired run. All six gated metrics are recorded
/// raw; no derived numbers are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RerankAbMetrics {
    pub paraphrase_mrr_at_10: f64,
    pub paraphrase_hit_at_5: f64,
    pub associative_mrr_at_10: f64,
    pub associative_hit_at_5: f64,
    pub combined_mrr_at_10: f64,
    pub combined_hit_at_5: f64,
}

/// Paired A/B artifact. Every provenance field is mandatory so a promote
/// decision is auditable (commit, dataset hash, model manifest, config).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RerankAbArtifact {
    pub schema_version: u32,
    pub commit: String,
    pub dataset_sha256: String,
    pub model_manifest_sha256: String,
    pub preset: String,
    pub top_n: usize,
    pub top_k: usize,
    /// Preregistered before results are inspected.
    pub primary_metric: RerankPrimaryMetric,
    pub off: RerankAbMetrics,
    pub on: RerankAbMetrics,
}

pub const RERANK_AB_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub enum RerankGateVerdict {
    Promote,
    Blocked(Vec<String>),
}

impl RerankGateVerdict {
    pub fn promoted(&self) -> bool {
        matches!(self, Self::Promote)
    }
}

/// Evaluate the promote gate for a paired artifact.
///
/// Rules (GH-851 B-012):
/// - paraphrase and associative MRR@10 / Hit@5 must each be non-regressing;
/// - both combined metrics must be non-regressing;
/// - the preregistered primary metric must improve by at least `0.05`
///   absolute.
///
/// Metadata must be complete; incomplete provenance blocks promotion.
pub fn evaluate_rerank_promote_gate(artifact: &RerankAbArtifact) -> Result<RerankGateVerdict> {
    if artifact.schema_version != RERANK_AB_SCHEMA_VERSION {
        bail!(
            "unsupported rerank A/B artifact schema {}, expected {}",
            artifact.schema_version,
            RERANK_AB_SCHEMA_VERSION
        );
    }
    for (field, value) in [
        ("commit", &artifact.commit),
        ("dataset_sha256", &artifact.dataset_sha256),
        ("model_manifest_sha256", &artifact.model_manifest_sha256),
        ("preset", &artifact.preset),
    ] {
        if value.trim().is_empty() {
            bail!("rerank A/B artifact is missing required provenance field {field}");
        }
    }
    if artifact.top_k == 0 || artifact.top_n < artifact.top_k {
        bail!(
            "rerank A/B artifact has invalid top_n/top_k ({}/{})",
            artifact.top_n,
            artifact.top_k
        );
    }

    let mut blockers = Vec::new();
    let gated = [
        (
            "paraphrase_mrr_at_10",
            artifact.off.paraphrase_mrr_at_10,
            artifact.on.paraphrase_mrr_at_10,
        ),
        (
            "paraphrase_hit_at_5",
            artifact.off.paraphrase_hit_at_5,
            artifact.on.paraphrase_hit_at_5,
        ),
        (
            "associative_mrr_at_10",
            artifact.off.associative_mrr_at_10,
            artifact.on.associative_mrr_at_10,
        ),
        (
            "associative_hit_at_5",
            artifact.off.associative_hit_at_5,
            artifact.on.associative_hit_at_5,
        ),
        (
            "combined_mrr_at_10",
            artifact.off.combined_mrr_at_10,
            artifact.on.combined_mrr_at_10,
        ),
        (
            "combined_hit_at_5",
            artifact.off.combined_hit_at_5,
            artifact.on.combined_hit_at_5,
        ),
    ];
    for (name, off, on) in gated {
        if !off.is_finite() || !on.is_finite() {
            blockers.push(format!("metric {name} has a non-finite value"));
            continue;
        }
        if on < off {
            blockers.push(format!("metric {name} regressed: off={off:.4} on={on:.4}"));
        }
    }
    let primary_off = artifact.primary_metric.value(&artifact.off);
    let primary_on = artifact.primary_metric.value(&artifact.on);
    if primary_on - primary_off < PRIMARY_METRIC_MIN_DELTA {
        blockers.push(format!(
            "preregistered primary metric {} delta {:.4} is below the required {:.2} absolute improvement",
            artifact.primary_metric.label(),
            primary_on - primary_off,
            PRIMARY_METRIC_MIN_DELTA
        ));
    }
    if blockers.is_empty() {
        Ok(RerankGateVerdict::Promote)
    } else {
        Ok(RerankGateVerdict::Blocked(blockers))
    }
}

/// Parse an artifact from JSON produced by the A/B runner.
pub fn parse_rerank_ab_artifact(content: &str) -> Result<RerankAbArtifact> {
    serde_json::from_str(content).context("parse rerank A/B artifact JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(base: f64) -> RerankAbMetrics {
        RerankAbMetrics {
            paraphrase_mrr_at_10: base,
            paraphrase_hit_at_5: base,
            associative_mrr_at_10: base,
            associative_hit_at_5: base,
            combined_mrr_at_10: base,
            combined_hit_at_5: base,
        }
    }

    fn artifact(off: RerankAbMetrics, on: RerankAbMetrics) -> RerankAbArtifact {
        RerankAbArtifact {
            schema_version: RERANK_AB_SCHEMA_VERSION,
            commit: "abc123".into(),
            dataset_sha256: "d".repeat(64),
            model_manifest_sha256: "m".repeat(64),
            preset: "bge-reranker-base".into(),
            top_n: 50,
            top_k: 20,
            primary_metric: RerankPrimaryMetric::CombinedMrrAt10,
            off,
            on,
        }
    }

    #[test]
    fn promote_gate_passes_on_non_regression_and_primary_delta() -> Result<()> {
        let verdict = evaluate_rerank_promote_gate(&artifact(metrics(0.60), metrics(0.66)))?;

        assert!(verdict.promoted());
        Ok(())
    }

    #[test]
    fn promote_gate_blocks_when_a_slice_regresses() -> Result<()> {
        let mut on = metrics(0.70);
        on.paraphrase_hit_at_5 = 0.55;

        let verdict = evaluate_rerank_promote_gate(&artifact(metrics(0.60), on))?;

        match verdict {
            RerankGateVerdict::Blocked(blockers) => {
                assert!(blockers.iter().any(|b| b.contains("paraphrase_hit_at_5")));
            }
            RerankGateVerdict::Promote => panic!("regressed slice must block promotion"),
        }
        Ok(())
    }

    #[test]
    fn promote_gate_blocks_when_primary_delta_is_too_small() -> Result<()> {
        let verdict = evaluate_rerank_promote_gate(&artifact(metrics(0.60), metrics(0.63)))?;

        match verdict {
            RerankGateVerdict::Blocked(blockers) => {
                assert!(blockers.iter().any(|b| b.contains("primary metric")));
            }
            RerankGateVerdict::Promote => panic!("insufficient primary delta must block"),
        }
        Ok(())
    }

    #[test]
    fn promote_gate_rejects_missing_provenance() {
        let mut bad = artifact(metrics(0.60), metrics(0.66));
        bad.model_manifest_sha256 = String::new();

        let error = evaluate_rerank_promote_gate(&bad).unwrap_err();

        assert!(error.to_string().contains("model_manifest_sha256"));
    }

    #[test]
    fn promote_gate_rejects_non_finite_metric() -> Result<()> {
        let mut on = metrics(0.70);
        on.combined_mrr_at_10 = f64::NAN;

        let verdict = evaluate_rerank_promote_gate(&artifact(metrics(0.60), on))?;

        match verdict {
            RerankGateVerdict::Blocked(blockers) => {
                assert!(blockers.iter().any(|b| b.contains("non-finite")));
            }
            RerankGateVerdict::Promote => panic!("non-finite metric must block"),
        }
        Ok(())
    }

    #[test]
    fn artifact_round_trips_through_json() -> Result<()> {
        let original = artifact(metrics(0.60), metrics(0.66));

        let parsed = parse_rerank_ab_artifact(&serde_json::to_string(&original)?)?;

        assert_eq!(parsed, original);
        Ok(())
    }
}
