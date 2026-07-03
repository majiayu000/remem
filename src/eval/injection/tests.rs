use std::path::Path;

use anyhow::Result;

use super::types::CORPUS_NAME;
use super::{
    run::rendered_line_contains_title_and_label, run_sandbox_eval, InjectionEvalMetadata,
    InjectionEvalOptions, InjectionEvalReport, InjectionMetricSummary, InjectionRateMetric,
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
    assert!(
        report.metrics.stale_anchor_labeling.is_perfect(),
        "{:#?}",
        report
    );
    assert!(
        report.metrics.user_prompt_submit_memory_recall.is_perfect(),
        "{:#?}",
        report
    );
    assert!(
        report
            .metrics
            .user_prompt_submit_abstention_false_positive_bound
            .is_perfect(),
        "{:#?}",
        report
    );
    assert_eq!(
        report.metadata.render_contract_version,
        crate::context::RENDER_CONTRACT_VERSION
    );
    assert!(
        report.metrics.block_churn_unchanged.is_perfect(),
        "{:#?}",
        report
    );
    assert!(
        report
            .metrics
            .block_churn_one_added_prefix_preserved
            .is_perfect(),
        "{:#?}",
        report
    );
    assert_eq!(report.churn.unchanged_changed_bytes, 0);
    assert!(report.churn.one_added_changed_bytes > 0);
    assert!(report.churn.one_added_prefix_preserved);
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
            render_contract_version: crate::context::RENDER_CONTRACT_VERSION,
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
            stale_anchor_labeling: InjectionRateMetric::new(1, 1),
            user_prompt_submit_memory_recall: InjectionRateMetric::new(1, 1),
            user_prompt_submit_abstention_false_positive_bound: InjectionRateMetric::new(1, 1),
            block_churn_unchanged: InjectionRateMetric::new(1, 1),
            block_churn_one_added_prefix_preserved: InjectionRateMetric::new(1, 1),
            all_checks_passed: true,
        },
        churn: super::types::InjectionChurnReport {
            unchanged_changed_bytes: 0,
            one_added_changed_bytes: 42,
            one_added_first_affected_section: Some("## Core".to_string()),
            one_added_prefix_preserved: true,
        },
        cases: vec![],
        failing_examples: vec![],
    };

    let rendered = format!("{report}");

    assert!(rendered.contains("=== remem eval-injection"));
    assert!(rendered.contains("expected_memory_recall: 2/2"));
    assert!(rendered.contains("forbidden_memory_exclusion: 3/3"));
    assert!(rendered.contains("abstention_false_positive_bound: 1/1"));
    assert!(rendered.contains("stale_anchor_labeling: 1/1"));
    assert!(rendered.contains("user_prompt_submit_memory_recall: 1/1"));
    assert!(rendered.contains("user_prompt_submit_abstention_false_positive_bound: 1/1"));
    assert!(rendered.contains("block_churn_unchanged: 1/1"));
    assert!(rendered.contains("block_churn_one_added_prefix_preserved: 1/1"));
    assert!(rendered.contains("render_contract_version="));
    assert!(rendered.contains("one_added_changed_bytes=42"));
    assert!(rendered.contains("all_checks_passed: true"));
}

#[test]
fn stale_anchor_eval_requires_label_on_target_memory_line() {
    let output = "\
**#8 Stale source anchor decision** (src=memory:#8;status=active;staleness=fresh;source_anchor=tracked) | **#9 Other memory** (src=memory:#9;status=active;staleness=fresh;source_anchor=verify-before-trust)
";

    assert!(!rendered_line_contains_title_and_label(
        output,
        8,
        "Stale source anchor decision",
        "source_anchor=verify-before-trust"
    ));
    assert!(rendered_line_contains_title_and_label(
        "**#8 Stale source anchor decision** (src=memory:#8;source_anchor=verify-before-trust)",
        8,
        "Stale source anchor decision",
        "source_anchor=verify-before-trust"
    ));
}
