use std::fmt::{Display, Formatter, Result as FmtResult};

use super::types::{GoldenEvalReport, MetricAverages};

impl Display for GoldenEvalReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(
            f,
            "remem eval — deterministic retrieval layer, {} queries, k={}, rank_k={}",
            self.total_queries, self.k, self.rank_k
        )?;
        if let Some(version) = self.version.as_deref() {
            writeln!(f, "schema: {version}")?;
        }
        if let Some(description) = self.description.as_deref() {
            writeln!(f, "{description}")?;
        }
        writeln!(f)?;
        writeln!(f, "--- Evaluation Layers ---")?;
        writeln!(
            f,
            "  retrieval: {}",
            self.evaluation_layers.retrieval.status
        )?;
        writeln!(
            f,
            "  answer_generation: {}",
            self.evaluation_layers.answer_generation.status
        )?;
        writeln!(
            f,
            "  llm_judge: {}",
            self.evaluation_layers.llm_judge.status
        )?;
        writeln!(f)?;

        for query in &self.queries {
            if let Some(metrics) = query.metrics.as_ref() {
                writeln!(
                    f,
                    "  [{}] {:>4} | H@{}={:.2} MRR@10={:.2} P@{}={:.2} R@{}={:.2} nDCG@10={:.2} evidence@{}={:.2} | {} | {}",
                    query.id,
                    query.status.label(),
                    self.k,
                    metrics.hit_at_k,
                    metrics.mrr_at_10,
                    self.k,
                    metrics.precision_at_k,
                    self.k,
                    metrics.recall_at_k,
                    metrics.ndcg_at_10,
                    self.k,
                    metrics.evidence_recall_at_k,
                    query.category,
                    query.query
                )?;
                write_failure_details(
                    f,
                    query.status,
                    &query.retrieved_ids,
                    &query.expected_relevant_ids,
                    &query.missing_relevant_ids,
                    query.missing_evidence_refs.len(),
                )?;
            } else {
                writeln!(
                    f,
                    "  [{}] {:>4} | results={} expected_refs={} matched_refs={} | {} | {}",
                    query.id,
                    query.status.label(),
                    query.result_count,
                    query.expected_refs,
                    query.matched_refs,
                    query.category,
                    query.query
                )?;
                write_failure_details(
                    f,
                    query.status,
                    &query.retrieved_ids,
                    &query.expected_relevant_ids,
                    &query.missing_relevant_ids,
                    query.missing_evidence_refs.len(),
                )?;
            }
        }

        writeln!(f)?;
        writeln!(
            f,
            "--- Retrieval Overall ({} scored, {} abstention, {} skipped) ---",
            self.scored_queries, self.abstention_queries, self.skipped_queries
        )?;
        if let Some(metrics) = self.overall.as_ref() {
            write_metrics(f, "all", metrics, self.k)?;
        }
        if self.abstention_queries > 0 {
            writeln!(
                f,
                "  Abstention: {}/{}",
                self.abstention_passed, self.abstention_queries
            )?;
        }

        writeln!(f)?;
        writeln!(f, "--- Answer/Judge Layer ---")?;
        writeln!(f, "  not run in deterministic golden retrieval eval")?;

        if !self.by_category.is_empty() {
            writeln!(f)?;
            writeln!(f, "--- By Category ---")?;
            for (category, evaluation) in &self.by_category {
                if let Some(metrics) = evaluation.metrics.as_ref() {
                    write_metrics(f, category, metrics, self.k)?;
                } else {
                    writeln!(
                        f,
                        "  {}: scored=0 abstention={}/{} total={}",
                        category,
                        evaluation.abstention_passed,
                        evaluation.abstention_queries,
                        evaluation.total_queries
                    )?;
                }
            }
        }
        Ok(())
    }
}

fn write_failure_details(
    f: &mut Formatter<'_>,
    status: super::types::QueryStatus,
    retrieved_ids: &[i64],
    expected_relevant_ids: &[i64],
    missing_relevant_ids: &[i64],
    missing_evidence_ref_count: usize,
) -> FmtResult {
    if !matches!(
        status,
        super::types::QueryStatus::Miss | super::types::QueryStatus::Fail
    ) {
        return Ok(());
    }
    writeln!(
        f,
        "       retrieved_ids={:?} expected_relevant_ids={:?} missing_relevant_ids={:?} missing_evidence_refs={}",
        retrieved_ids, expected_relevant_ids, missing_relevant_ids, missing_evidence_ref_count
    )
}

fn write_metrics(
    f: &mut Formatter<'_>,
    label: &str,
    metrics: &MetricAverages,
    k: usize,
) -> FmtResult {
    writeln!(
        f,
        "  {}: n={} H@{}={:.3} MRR@10={:.3} P@{}={:.3} R@{}={:.3} nDCG@10={:.3} evidence@{}={:.3}",
        label,
        metrics.count,
        k,
        metrics.hit_at_k,
        metrics.mrr_at_10,
        k,
        metrics.precision_at_k,
        k,
        metrics.recall_at_k,
        metrics.ndcg_at_10,
        k,
        metrics.evidence_recall_at_k
    )
}
