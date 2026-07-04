use std::fmt::{Display, Formatter, Result as FmtResult};

use super::ProviderComparisonReport;

impl Display for ProviderComparisonReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(
            f,
            "remem provider comparison: decision={:?}, change_default={}, k={}",
            self.default_decision.decision, self.default_decision.change_default, self.k
        )?;
        writeln!(f, "reason: {}", self.default_decision.decision_reason)?;
        for row in &self.providers {
            if row.available {
                let slice_metrics = row
                    .provider_comparison_slice
                    .as_ref()
                    .and_then(|slice| slice.metrics.as_ref());
                let evidence = slice_metrics.map_or(0.0, |metrics| metrics.evidence_recall_at_k);
                writeln!(
                    f,
                    "- {} available model={} evidence@{}={:.3} query_embed_p95_ms={:.2}",
                    row.provider,
                    row.model_id.as_deref().unwrap_or("unknown"),
                    self.k,
                    evidence,
                    row.query_embedding_latency_p95_ms.unwrap_or_default()
                )?;
            } else {
                writeln!(
                    f,
                    "- {} unavailable: {}",
                    row.provider,
                    row.unavailable_reason.as_deref().unwrap_or("unknown")
                )?;
            }
        }
        Ok(())
    }
}
