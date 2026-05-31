use anyhow::Result;

use super::{run_sandbox_eval, GovernanceEvalOptions, LifecycleCounts};

#[test]
fn governance_eval_sandbox_reports_all_required_metrics() -> Result<()> {
    let report = run_sandbox_eval(GovernanceEvalOptions { k: 5 })?;

    assert!(!report.metadata.real_db_touched);
    assert_eq!(report.metadata.storage, "in-memory sqlite");
    assert!(!std::path::Path::new(&report.metadata.data_dir).exists());
    assert!(report.metrics.owner_routing_accuracy.is_perfect());
    assert!(report.metrics.evidence_recall_at_k.is_perfect());
    assert!(report.metrics.active_current_precision.is_perfect());
    assert!(report.metrics.stale_exclusion_rate.is_perfect());
    assert!(report.metrics.context_injection_precision.is_perfect());
    assert!(report.metrics.all_checks_passed);
    assert!(report.failing_examples.is_empty());
    assert_eq!(
        report.lifecycle_counts,
        LifecycleCounts {
            add: 1,
            update: 1,
            invalidate: 1,
            noop: 1,
            defer: 1
        }
    );
    assert_eq!(report.summary_candidates.total, 7);
    assert_eq!(report.summary_candidates.pending_review, 7);
    assert_eq!(report.summary_candidates.auto_promoted, 0);
    assert_eq!(report.summary_candidates.active_summary_memories, 0);
    Ok(())
}

#[test]
fn governance_eval_context_excludes_pollution_and_stale_rows() -> Result<()> {
    let report = run_sandbox_eval(GovernanceEvalOptions { k: 5 })?;

    assert!(report
        .context
        .included_topic_keys
        .contains(&"repo-dnd-pointer".to_string()));
    assert!(report
        .context
        .included_topic_keys
        .contains(&"repo-codex-file-exception".to_string()));
    assert!(report
        .context
        .excluded_owner_titles
        .contains(&"Codex approval mode".to_string()));
    assert!(report
        .context
        .excluded_owner_titles
        .contains(&"Grok API image references".to_string()));
    assert!(report.context.forbidden_titles.is_empty());
    assert_eq!(report.context.unsafe_owner_included, 0);
    Ok(())
}
