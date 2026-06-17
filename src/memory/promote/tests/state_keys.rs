use anyhow::Result;
use rusqlite::Connection;

use super::super::promote_summary_to_memory_candidates;
use super::promote::{record_summary_evidence, setup_conn};
use crate::memory_candidate::review::approve_candidate;

const DECISION_STATE_KEY: &str = "decision-cjk-fts5-search-tokenizer-trigram";

fn approve_latest_pending_candidate(conn: &mut Connection) -> Result<i64> {
    let candidate_id: i64 = conn.query_row(
        "SELECT id
         FROM memory_candidates
         WHERE review_status = 'pending_review'
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    approve_candidate(conn, candidate_id)?
        .ok_or_else(|| anyhow::anyhow!("candidate should promote to memory"))
}

#[test]
fn summary_decision_candidate_uses_semantic_state_topic_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-decision-state-key";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize CJK search"),
        Some("Use FTS5 trigram tokenizer for CJK text search support"),
        None,
        None,
    )?;
    assert_eq!(count, 1);

    let (topic_key, state_key, state_key_confidence, state_key_reason): (
        String,
        String,
        f64,
        String,
    ) = conn.query_row(
        "SELECT topic_key, state_key, state_key_confidence, state_key_reason
         FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(topic_key, DECISION_STATE_KEY);
    assert_eq!(state_key, DECISION_STATE_KEY);
    assert_eq!(state_key_confidence, 1.0);
    assert_eq!(state_key_reason, "stable_topic_key");
    Ok(())
}

#[test]
fn summary_decision_paraphrases_promote_through_same_state_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let project = "test/proj";

    record_summary_evidence(&conn, "session-decision-state-a", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-decision-state-a",
        project,
        Some("Optimize CJK search"),
        Some("Use FTS5 trigram tokenizer for CJK text search support"),
        None,
        None,
    )?;
    let first_memory_id = approve_latest_pending_candidate(&mut conn)?;

    record_summary_evidence(&conn, "session-decision-state-b", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-decision-state-b",
        project,
        Some("Refine CJK search"),
        Some("Switch CJK search to FTS5 trigram tokenization."),
        None,
        None,
    )?;
    let second_topic_key: String = conn.query_row(
        "SELECT topic_key
         FROM memory_candidates
         WHERE review_status = 'pending_review'
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let second_memory_id = approve_latest_pending_candidate(&mut conn)?;

    let first_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [first_memory_id],
        |row| row.get(0),
    )?;
    let (second_status, state_key, current_memory_id): (String, String, i64) = conn.query_row(
        "SELECT m.status, sk.state_key, sk.current_memory_id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.id = ?1",
        [second_memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memories
         WHERE memory_type = 'decision' AND status = 'active'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(second_topic_key, DECISION_STATE_KEY);
    assert_eq!(first_status, "stale");
    assert_eq!(second_status, "active");
    assert_eq!(state_key, DECISION_STATE_KEY);
    assert_eq!(current_memory_id, second_memory_id);
    assert_eq!(active_count, 1);
    Ok(())
}

#[test]
fn summary_preference_paraphrases_promote_through_same_state_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let project = "test/proj";

    record_summary_evidence(&conn, "session-pref-state-a", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-pref-state-a",
        project,
        Some("Capture preference"),
        None,
        None,
        Some("Keep verification status separate from data and code changes."),
    )?;
    let first_memory_id = approve_latest_pending_candidate(&mut conn)?;

    record_summary_evidence(&conn, "session-pref-state-b", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-pref-state-b",
        project,
        Some("Capture refined preference"),
        None,
        None,
        Some("Report data and code changes separately from verification status."),
    )?;
    let second_topic_key: String = conn.query_row(
        "SELECT topic_key
         FROM memory_candidates
         WHERE review_status = 'pending_review'
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let second_memory_id = approve_latest_pending_candidate(&mut conn)?;

    let first_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [first_memory_id],
        |row| row.get(0),
    )?;
    let (second_status, state_key, current_memory_id): (String, String, i64) = conn.query_row(
        "SELECT m.status, sk.state_key, sk.current_memory_id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.id = ?1",
        [second_memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memories
         WHERE memory_type = 'preference' AND status = 'active'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(second_topic_key, "verification-status-separation");
    assert_eq!(first_status, "stale");
    assert_eq!(second_status, "active");
    assert_eq!(state_key, "verification-status-separation");
    assert_eq!(current_memory_id, second_memory_id);
    assert_eq!(active_count, 1);
    Ok(())
}

#[test]
fn summary_workflow_preference_variants_promote_through_same_state_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let project = "test/proj";

    record_summary_evidence(&conn, "session-small-change-a", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-small-change-a",
        project,
        Some("Capture workflow preference"),
        None,
        None,
        Some("Prefers small, reversible changes (“一处改动一个提交”) with concrete verification output: tests, lint, job IDs."),
    )?;
    let first_memory_id = approve_latest_pending_candidate(&mut conn)?;

    record_summary_evidence(&conn, "session-small-change-b", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-small-change-b",
        project,
        Some("Capture refined workflow preference"),
        None,
        None,
        Some("Prefers one change per commit with concrete evidence from tests, build artifacts, and checklist proof."),
    )?;
    let second_topic_key: String = conn.query_row(
        "SELECT topic_key
         FROM memory_candidates
         WHERE review_status = 'pending_review'
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let second_memory_id = approve_latest_pending_candidate(&mut conn)?;

    let first_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [first_memory_id],
        |row| row.get(0),
    )?;
    let (second_status, state_key, current_memory_id): (String, String, i64) = conn.query_row(
        "SELECT m.status, sk.state_key, sk.current_memory_id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.id = ?1",
        [second_memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memories
         WHERE memory_type = 'preference' AND status = 'active'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(second_topic_key, "small-reversible-verified-changes");
    assert_eq!(first_status, "stale");
    assert_eq!(second_status, "active");
    assert_eq!(state_key, "small-reversible-verified-changes");
    assert_eq!(current_memory_id, second_memory_id);
    assert_eq!(active_count, 1);
    Ok(())
}

#[test]
fn summary_generic_preference_paraphrases_consolidate_without_domain_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let project = "test/proj";

    record_summary_evidence(&conn, "session-generic-pref-a", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-generic-pref-a",
        project,
        Some("Capture communication preference"),
        None,
        None,
        Some("Prefer concise Chinese progress updates."),
    )?;
    let first_topic_key: String = conn.query_row(
        "SELECT topic_key
         FROM memory_candidates
         WHERE review_status = 'pending_review'
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let first_memory_id = approve_latest_pending_candidate(&mut conn)?;

    record_summary_evidence(&conn, "session-generic-pref-b", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-generic-pref-b",
        project,
        Some("Capture communication preference refinement"),
        None,
        None,
        Some("Prefer brief Chinese status notes."),
    )?;
    let second_topic_key: String = conn.query_row(
        "SELECT topic_key
         FROM memory_candidates
         WHERE review_status = 'pending_review'
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let second_memory_id = approve_latest_pending_candidate(&mut conn)?;

    let first_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [first_memory_id],
        |row| row.get(0),
    )?;
    let second_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [second_memory_id],
        |row| row.get(0),
    )?;
    let (operation, reason): (String, String) = conn.query_row(
        "SELECT operation, reason
         FROM memory_operation_log
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memories
         WHERE memory_type = 'preference' AND status = 'active'",
        [],
        |row| row.get(0),
    )?;

    assert_ne!(first_topic_key, second_topic_key);
    assert!(first_topic_key.starts_with("preference-"));
    assert!(second_topic_key.starts_with("preference-"));
    assert_eq!(first_status, "stale");
    assert_eq!(second_status, "active");
    assert_eq!(operation, "update");
    assert!(reason.contains("generic preference consolidation kind=refinement"));
    assert_eq!(active_count, 1);
    Ok(())
}

#[test]
fn summary_decision_contradiction_supersedes_same_state_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let project = "test/proj";

    record_summary_evidence(&conn, "session-decision-conflict-a", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-decision-conflict-a",
        project,
        Some("Choose local vector recall"),
        Some("Use SQLite vector embeddings for local semantic recall"),
        None,
        None,
    )?;
    let first_memory_id = approve_latest_pending_candidate(&mut conn)?;

    record_summary_evidence(&conn, "session-decision-conflict-b", project)?;
    promote_summary_to_memory_candidates(
        &mut conn,
        "session-decision-conflict-b",
        project,
        Some("Reverse local vector recall"),
        Some("Stop using SQLite vector embeddings for local semantic recall."),
        None,
        None,
    )?;
    let second_memory_id = approve_latest_pending_candidate(&mut conn)?;

    let first_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [first_memory_id],
        |row| row.get(0),
    )?;
    let supersedes_edge_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_edges
         WHERE edge_type = 'supersedes'
           AND from_memory_id = ?1
           AND to_memory_id = ?2",
        [first_memory_id, second_memory_id],
        |row| row.get(0),
    )?;

    assert_eq!(first_status, "stale");
    assert_eq!(supersedes_edge_count, 1);
    Ok(())
}

#[test]
fn summary_learned_candidates_use_semantic_state_topic_keys_when_possible() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-learned-state-keys";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let learned = "- FTS5 trigram tokenizer handles CJK query recall\n\
                   - Root cause: warning-only fallback hid missing data; avoid silent degradation.";
    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Capture durable learned items"),
        None,
        Some(learned),
        None,
    )?;
    assert_eq!(count, 2);

    let rows = conn
        .prepare(
            "SELECT memory_type, topic_key, state_key
             FROM memory_candidates
             ORDER BY memory_type ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    assert_eq!(rows.len(), 2);
    for (memory_type, topic_key, state_key) in rows {
        assert!(
            topic_key.starts_with(&format!("{memory_type}-")),
            "topic key should keep type prefix: {topic_key}"
        );
        let tail = topic_key
            .rsplit_once('-')
            .map(|(_, tail)| tail)
            .unwrap_or("");
        assert!(
            !(tail.len() >= 8 && tail.chars().all(|ch| ch.is_ascii_hexdigit())),
            "summary topic key should not be content-hash-like: {topic_key}"
        );
        assert_eq!(state_key, topic_key);
    }
    Ok(())
}
