use anyhow::Result;
use rusqlite::Connection;

use super::{
    run_eval, DedupReport, EvalReport, ProjectLeakReport, SelfRetrievalReport, TitleQualityReport,
};

#[test]
fn eval_local_empty_db_reports_zeroes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(crate::migrate::MIGRATIONS[0].sql)?;

    let report = run_eval(&conn)?;

    assert_eq!(report.total_memories, 0);
    assert_eq!(report.dedup.duplicate_count, 0);
    assert_eq!(report.project_leak.total_tested, 0);
    assert_eq!(report.title_quality.total, 0);
    assert_eq!(report.self_retrieval.total_tested, 0);
    Ok(())
}

#[test]
fn eval_report_display_includes_overall_score() {
    let report = EvalReport {
        total_memories: 10,
        dedup: DedupReport {
            duplicate_groups: 1,
            duplicate_count: 2,
            duplicate_rate: 0.2,
            worst_groups: vec![("duplicate preview".to_string(), 3)],
        },
        project_leak: ProjectLeakReport {
            total_tested: 5,
            leaked: 1,
            leak_rate: 0.2,
        },
        title_quality: TitleQualityReport {
            total: 10,
            bullet_prefix: 1,
            too_long: 2,
            bullet_rate: 0.1,
        },
        self_retrieval: SelfRetrievalReport {
            total_tested: 4,
            found: 3,
            retrieval_rate: 0.75,
        },
    };

    let rendered = format!("{}", report);

    assert!(rendered.contains("=== remem eval-local (10 memories) ==="));
    assert!(rendered.contains("[dedup] 2 duplicates in 1 groups"));
    assert!(rendered.contains("[project_filter] tested 5 entities, 1 leaked"));
    assert!(rendered.contains("[title_quality]"));
    assert!(rendered.contains("[self_retrieval] 3/4"));
    assert!(rendered.contains("--- overall:"));
}
