use anyhow::Result;

use super::super::promote_summary_to_memory_candidates;
use super::promote::{record_summary_evidence, record_summary_evidence_with_content, setup_conn};

#[test]
fn test_summary_candidates_multi_decisions_do_not_create_memories() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-decisions";
    let project = "test/proj";
    let evidence_id = record_summary_evidence(&conn, session_id, project)?;

    let decisions = "• Use RwLock instead of Mutex for concurrent read support\n\
                     • Switch to Unicode segmenter for CJK text search\n\
                     • Set compression threshold to 100 observations";
    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize search and concurrency"),
        Some(decisions),
        None,
        None,
    )?;
    assert_eq!(count, 3);

    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let candidate_rows = conn
        .prepare(
            "SELECT memory_type, review_status, evidence_event_ids, source_kind,
                    auto_promote_block_reason
             FROM memory_candidates
             ORDER BY id ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let evidence_json = serde_json::to_string(&vec![evidence_id])?;

    assert_eq!(memory_count, 0);
    assert_eq!(candidate_rows.len(), 3);
    assert!(candidate_rows.iter().all(|row| {
        row.0 == "decision"
            && row.1 == "pending_review"
            && row.2 == evidence_json
            && row.3 == "summary"
            && row.4 == "summary_source_support_failed"
    }));
    Ok(())
}

#[test]
fn summary_decision_shadow_gate_records_would_promote_without_active_memory() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-summary-shadow";
    let project = "test/proj";
    let decision = "Use source kind telemetry for summary promotion gate";
    record_summary_evidence_with_content(&conn, "codex-cli", session_id, project, decision)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        None,
        Some(decision),
        None,
        None,
    )?;
    assert_eq!(count, 1);

    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let (source_kind, review_status, block_reason): (String, String, String) = conn.query_row(
        "SELECT source_kind, review_status, auto_promote_block_reason
         FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    assert_eq!(memory_count, 0);
    assert_eq!(source_kind, "summary");
    assert_eq!(review_status, "pending_review");
    assert_eq!(block_reason, "summary_gate_shadow");
    Ok(())
}
