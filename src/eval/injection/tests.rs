use std::path::Path;

use anyhow::Result;

use super::types::CORPUS_NAME;
use super::{
    run_sandbox_eval, InjectionEvalMetadata, InjectionEvalOptions, InjectionEvalReport,
    InjectionMetricSummary, InjectionRateMetric,
};

#[test]
fn injection_eval_exercises_session_start_render_path() -> Result<()> {
    let report = run_sandbox_eval(InjectionEvalOptions::default())?;

    assert!(!report.metadata.real_db_touched);
    assert_eq!(report.metadata.boundary, "context::render_context_output");
    assert!(
        report.metrics.expected_memory_recall.is_perfect(),
        "{:#?}",
        report
    );
    assert!(
        report.metrics.forbidden_memory_exclusion.is_perfect(),
        "{:#?}",
        report
    );
    assert!(
        report.metrics.abstention_false_positive_bound.is_perfect(),
        "{:#?}",
        report
    );
    assert!(report.metrics.all_checks_passed, "{:#?}", report);
    assert!(report.metadata.memories_loaded > 0);
    assert!(report.metadata.core_count > 0 || report.metadata.index_count > 0);
    assert!(report.failing_examples.is_empty());
    assert!(!Path::new(&report.metadata.data_dir).exists());
    Ok(())
}

#[test]
fn injection_eval_display_includes_metrics() {
    let report = InjectionEvalReport {
        metadata: InjectionEvalMetadata {
            corpus: CORPUS_NAME.to_string(),
            boundary: "context::render_context_output".to_string(),
            storage: "temporary sqlite".to_string(),
            data_dir: "/tmp/example".to_string(),
            data_dir_kept: false,
            real_db_touched: false,
            project: "/tmp/remem-injection-eval/repo".to_string(),
            host: "codex-cli".to_string(),
            branch: "main".to_string(),
            output_chars: 100,
            memories_loaded: 2,
            core_count: 1,
            index_count: 1,
            lesson_count: 0,
            preference_count: 0,
            session_count: 0,
            workstream_count: 0,
            truncated: false,
        },
        metrics: InjectionMetricSummary {
            expected_memory_recall: InjectionRateMetric::new(2, 2),
            forbidden_memory_exclusion: InjectionRateMetric::new(3, 3),
            abstention_false_positive_bound: InjectionRateMetric::new(1, 1),
            all_checks_passed: true,
        },
        cases: vec![],
        failing_examples: vec![],
    };

    let rendered = format!("{report}");

    assert!(rendered.contains("=== remem eval-injection"));
    assert!(rendered.contains("expected_memory_recall: 2/2"));
    assert!(rendered.contains("forbidden_memory_exclusion: 3/3"));
    assert!(rendered.contains("abstention_false_positive_bound: 1/1"));
    assert!(rendered.contains("all_checks_passed: true"));
}
