use rusqlite::{params, Connection};

use crate::db::test_support::ScopedTestDataDir;
use crate::{db, memory};

use super::super::database::{
    check_declared_empty_surfaces, check_promotion_funnel, check_temporal_facts,
};

fn seed_review_gated_candidate(conn: &Connection, project: &str) -> anyhow::Result<()> {
    let capture = db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "session-review-gated",
            project,
            cwd: Some(project),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content: "candidate evidence",
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
    )?;
    let (project_id, session_row_id): (i64, i64) = conn.query_row(
        "SELECT project_id, session_row_id FROM captured_events WHERE id = ?1",
        [capture.event_row_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let evidence_json = serde_json::to_string(&vec![capture.event_row_id])?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO observations
         (memory_session_id, project, type, observation_type, text, narrative,
          evidence_event_ids, confidence, session_row_id, status, created_at_epoch)
         VALUES ('session-review-gated', ?1, 'decision', 'decision',
                 'Use review approval to promote this supported candidate.',
                 'Use review approval to promote this supported candidate.',
                 ?2, 0.9, ?3, 'active', ?4)",
        params![project, evidence_json, session_row_id, now],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'review-gated-fact',
                 'Use review approval to promote this supported candidate.',
                 ?2, 0.9, 'medium', 'pending_review', ?3, ?3)",
        params![project_id, evidence_json, now],
    )?;
    Ok(())
}

#[test]
fn promotion_funnel_points_all_pending_candidates_to_review_flow() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-promotion-funnel-review-action");
    let conn = db::open_db()?;
    seed_review_gated_candidate(&conn, "/repo/a")?;

    let check = check_promotion_funnel(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("promotion is review-gated"),
        "{}",
        check.detail
    );
    assert!(
        check
            .detail
            .contains("remem review list --project /repo/a --limit 20"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains("remem review approve <id>"),
        "{}",
        check.detail
    );
    assert!(check.detail.contains("non-duplicate"), "{}", check.detail);
    assert!(
        check.detail.contains("linked temporal fact"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn promotion_funnel_reports_summary_shadow_source_split() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-summary-shadow-source-split");
    let conn = db::open_db()?;
    seed_review_gated_candidate(&conn, "/repo/a")?;
    conn.execute(
        "UPDATE memory_candidates
         SET source_kind = 'summary',
             auto_promote_block_reason = 'summary_gate_shadow'",
        [],
    )?;

    let check = check_promotion_funnel(Some(&conn));

    assert!(
        check
            .detail
            .contains("candidate_sources=summary:total=1,pending=1,shadow_would_promote=1"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn declared_empty_surfaces_defers_zero_facts_cause_to_temporal_check() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-declared-empty-review-gated");
    let conn = db::open_db()?;
    seed_review_gated_candidate(&conn, "/repo/a")?;

    let check = check_declared_empty_surfaces(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check
            .detail
            .contains("memory_facts=0; check Temporal facts"),
        "{}",
        check.detail
    );
    assert!(
        !check.detail.contains("memory_facts=0 despite"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn temporal_facts_distinguishes_review_gated_candidates_from_extraction_gap() -> anyhow::Result<()>
{
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-review-gated");
    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-review-gated"),
        "/repo/a",
        None,
        "source memory",
        "A source memory exists while candidates wait for review.",
        "decision",
        None,
    )?;
    seed_review_gated_candidate(&conn, "/repo/a")?;

    let check = check_temporal_facts(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check.detail.contains("memory_facts has 0 row(s)"),
        "{}",
        check.detail
    );
    assert!(
        check.detail.contains("promotion is review-gated"),
        "{}",
        check.detail
    );
    assert!(
        !check
            .detail
            .contains("production fact extraction is not populating"),
        "{}",
        check.detail
    );
    Ok(())
}

#[test]
fn temporal_facts_keeps_mixed_promoted_and_pending_candidates_on_extraction_gap(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("doctor-temporal-facts-mixed-candidates");
    let conn = db::open_db()?;
    memory::insert_memory(
        &conn,
        Some("session-mixed-candidates"),
        "/repo/a",
        None,
        "promoted source memory",
        "A promoted memory exists while another candidate waits for review.",
        "decision",
        None,
    )?;
    seed_review_gated_candidate(&conn, "/repo/a")?;
    conn.execute(
        "INSERT INTO memory_candidates
         (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES ('project', 'decision', 'already-promoted',
                 'A promoted candidate already exists.', '[]', 0.9,
                 'low', 'approved', 1, 1)",
        [],
    )?;

    let check = check_temporal_facts(Some(&conn));

    assert_eq!(check.icon(), "WARN");
    assert!(
        check
            .detail
            .contains("production fact extraction is not populating"),
        "{}",
        check.detail
    );
    assert!(
        !check.detail.contains("promotion is review-gated"),
        "{}",
        check.detail
    );
    Ok(())
}
