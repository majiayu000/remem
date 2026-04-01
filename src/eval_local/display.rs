use crate::eval_local::types::EvalReport;

const MAX_GOOD_TITLE_LEN: usize = 120;

impl std::fmt::Display for EvalReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "=== remem eval-local ({} memories) ===\n",
            self.total_memories
        )?;
        writeln!(
            f,
            "[dedup] {} duplicates in {} groups ({:.1}%)",
            self.dedup.duplicate_count,
            self.dedup.duplicate_groups,
            self.dedup.duplicate_rate * 100.0
        )?;
        if !self.dedup.worst_groups.is_empty() {
            writeln!(f, "  worst:")?;
            for (preview, count) in &self.dedup.worst_groups {
                writeln!(f, "    {}x  {}", count, preview)?;
            }
        }

        writeln!(
            f,
            "\n[project_filter] tested {} entities, {} leaked ({:.1}%)",
            self.project_leak.total_tested,
            self.project_leak.leaked,
            self.project_leak.leak_rate * 100.0
        )?;
        writeln!(
            f,
            "\n[title_quality] {:.1}% start with bullet, {:.1}% too long (>{} chars)",
            self.title_quality.bullet_rate * 100.0,
            title_too_long_rate(self),
            MAX_GOOD_TITLE_LEN
        )?;
        writeln!(
            f,
            "\n[self_retrieval] {}/{} ({:.1}%)",
            self.self_retrieval.found,
            self.self_retrieval.total_tested,
            self.self_retrieval.retrieval_rate * 100.0
        )?;
        writeln!(f, "\n--- overall: {:.1}/5.0 ---", self.overall_score())?;
        Ok(())
    }
}

impl EvalReport {
    pub fn overall_score(&self) -> f64 {
        let dedup_score = (1.0 - self.dedup.duplicate_rate).max(0.0) * 5.0;
        let leak_score = (1.0 - self.project_leak.leak_rate).max(0.0) * 5.0;
        let title_score = (1.0 - self.title_quality.bullet_rate).max(0.0) * 5.0;
        let retrieval_score = self.self_retrieval.retrieval_rate * 5.0;

        dedup_score * 0.30 + leak_score * 0.25 + title_score * 0.15 + retrieval_score * 0.30
    }
}

fn title_too_long_rate(report: &EvalReport) -> f64 {
    if report.title_quality.total > 0 {
        report.title_quality.too_long as f64 / report.title_quality.total as f64 * 100.0
    } else {
        0.0
    }
}
