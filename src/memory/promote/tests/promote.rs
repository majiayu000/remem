use anyhow::Result;
use rusqlite::Connection;

use super::super::promote_summary_to_memory_candidates;
use super::super::slug::content_hash;
use crate::db;
use crate::memory::service::{save_memory, SaveMemoryRequest};

pub(super) fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

pub(super) fn record_summary_evidence(
    conn: &Connection,
    session_id: &str,
    project: &str,
) -> Result<i64> {
    record_summary_evidence_with_content(
        conn,
        "codex-cli",
        session_id,
        project,
        "summary source payload",
    )
}

fn record_summary_evidence_with_host(
    conn: &Connection,
    host: &str,
    session_id: &str,
    project: &str,
) -> Result<i64> {
    record_summary_evidence_with_content(conn, host, session_id, project, "summary source payload")
}

fn record_summary_evidence_with_content(
    conn: &Connection,
    host: &str,
    session_id: &str,
    project: &str,
    content: &str,
) -> Result<i64> {
    let outcome = db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host,
            session_id,
            project,
            cwd: Some(project),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
    )?;
    Ok(outcome.event_row_id)
}

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

#[test]
fn test_summary_candidates_learned_lesson_and_discovery() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-learned";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let learned = "- FTS5 trigram tokenizer handles CJK without word boundaries\n\
                   - Root cause: warning-only fallback hid missing data; avoid silent degradation.";
    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Research storage"),
        None,
        Some(learned),
        None,
    )?;
    assert_eq!(count, 2);

    let rows = conn
        .prepare(
            "SELECT memory_type, confidence, review_status
             FROM memory_candidates
             ORDER BY memory_type ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "discovery");
    assert_eq!(rows[0].2, "pending_review");
    assert_eq!(rows[1].0, "lesson");
    assert!(rows[1].1 >= 0.8);
    assert_eq!(rows[1].2, "pending_review");
    Ok(())
}

#[test]
fn test_summary_candidate_duplicate_output_is_idempotent() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-dup";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let decision = "Use FTS5 trigram tokenizer for CJK text search support";
    let first = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize search"),
        Some(decision),
        None,
        None,
    )?;
    let second = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Optimize search"),
        Some(decision),
        None,
        None,
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(first, 1);
    assert_eq!(second, 0);
    assert_eq!(candidate_count, 1);
    Ok(())
}

#[test]
fn test_summary_preference_candidate_defaults_to_project_scope() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-preference";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Capture local preference"),
        None,
        None,
        Some("Always run project-specific smoke tests"),
    )?;
    assert_eq!(count, 1);

    let (scope, memory_type, owner_scope, owner_key): (String, String, String, String) = conn
        .query_row(
            "SELECT scope, memory_type, owner_scope, owner_key FROM memory_candidates",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    assert_eq!(scope, "project");
    assert_eq!(memory_type, "preference");
    assert_eq!(owner_scope, "repo");
    assert_eq!(owner_key, project);
    Ok(())
}

#[test]
fn test_summary_preference_candidate_uses_semantic_state_topic_key() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-preference-state-key";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Capture preference"),
        None,
        None,
        Some("Keep verification status separate from data and code changes."),
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
    assert_eq!(topic_key, "verification-status-separation");
    assert_eq!(state_key, "verification-status-separation");
    assert_eq!(state_key_confidence, 1.0);
    assert_eq!(state_key_reason, "stable_topic_key");
    Ok(())
}

#[test]
fn test_summary_candidates_missing_evidence_fails_closed() -> Result<()> {
    let mut conn = setup_conn()?;

    let err = promote_summary_to_memory_candidates(
        &mut conn,
        "missing-evidence",
        "test/proj",
        Some("Optimize search"),
        Some("Use FTS5 trigram tokenizer for CJK text search support"),
        None,
        None,
    )
    .expect_err("missing captured evidence should fail closed");

    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert!(err.to_string().contains("missing captured evidence"));
    assert_eq!(memory_count, 0);
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[test]
fn test_summary_candidate_content_keeps_compact_context() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-content";
    let project = "test/proj";
    record_summary_evidence(&conn, session_id, project)?;

    promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Fix search"),
        Some("Switched from unicode61 to trigram tokenizer for better CJK support"),
        None,
        None,
    )?;

    let text: String =
        conn.query_row("SELECT text FROM memory_candidates", [], |row| row.get(0))?;
    assert!(
        !text.contains("**Request**"),
        "content should not have boilerplate: {text}"
    );
    assert!(
        text.contains("[Context:"),
        "content should have compact context: {text}"
    );
    Ok(())
}

#[test]
fn summary_candidate_promotion_skips_candidate_covered_by_exact_session_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-claim-suppress";
    let project = "test/proj";
    let decision = "Use exact session memory claims to suppress duplicate summary candidates";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: decision.to_string(),
            title: Some("Session claim suppression".to_string()),
            project: Some(project.to_string()),
            session_id: Some(session_id.to_string()),
            host: Some("codex-cli".to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    let claim_id = saved
        .claim_id
        .ok_or_else(|| anyhow::anyhow!("save_memory should return claim_id"))?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        Some(decision),
        None,
        None,
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let noop: (i64, i64, String, String) = conn.query_row(
        "SELECT memory_claim_id, memory_id, reason, memory_type
         FROM memory_candidate_noops",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let consumed: (Option<i64>, Option<String>, Option<String>) = conn.query_row(
        "SELECT consumed_at_epoch, consumed_by_session_id, consumed_reason
         FROM memory_claims
         WHERE id = ?1",
        [claim_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;

    assert_eq!(count, 0);
    assert_eq!(candidate_count, 0);
    assert_eq!(noop.0, claim_id);
    assert_eq!(noop.1, saved.id);
    assert_eq!(noop.2, "covered_by_manual_save");
    assert_eq!(noop.3, "decision");
    assert!(consumed.0.is_some());
    assert_eq!(consumed.1.as_deref(), Some(session_id));
    assert_eq!(consumed.2.as_deref(), Some("covered_by_manual_save"));
    Ok(())
}

#[test]
fn summary_candidate_promotion_skips_candidate_covered_by_recent_project_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-claim-recent-fallback";
    let project = "test/proj";
    let decision = "Use recent project memory claims when exact session id is unavailable";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: decision.to_string(),
            title: Some("Recent claim fallback".to_string()),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    let claim_id = saved
        .claim_id
        .ok_or_else(|| anyhow::anyhow!("save_memory should return claim_id"))?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        Some(decision),
        None,
        None,
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let noop_session: String = conn.query_row(
        "SELECT session_id FROM memory_candidate_noops WHERE memory_claim_id = ?1",
        [claim_id],
        |row| row.get(0),
    )?;
    let consumed_by: Option<String> = conn.query_row(
        "SELECT consumed_by_session_id FROM memory_claims WHERE id = ?1",
        [claim_id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 0);
    assert_eq!(candidate_count, 0);
    assert_eq!(noop_session, session_id);
    assert_eq!(consumed_by.as_deref(), Some(session_id));
    Ok(())
}

#[test]
fn summary_preference_candidate_skips_exact_recent_claim_with_legacy_topic() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-preference-legacy-topic-claim";
    let project = "test/proj";
    let preference = "Keep verification status separate from data and code changes.";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: preference.to_string(),
            title: Some("Legacy preference".to_string()),
            project: Some(project.to_string()),
            topic_key: Some("legacy-preference-11111111".to_string()),
            memory_type: Some("preference".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    let claim_id = saved
        .claim_id
        .ok_or_else(|| anyhow::anyhow!("save_memory should return claim_id"))?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Capture preference"),
        None,
        None,
        Some(preference),
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let (noop_count, noop_reason): (i64, String) = conn.query_row(
        "SELECT COUNT(*), COALESCE(MAX(reason), '')
         FROM memory_candidate_noops
         WHERE memory_claim_id = ?1",
        [claim_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let consumed_by: Option<String> = conn.query_row(
        "SELECT consumed_by_session_id FROM memory_claims WHERE id = ?1",
        [claim_id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 0);
    assert_eq!(candidate_count, 0);
    assert_eq!(noop_count, 1);
    assert_eq!(noop_reason, "covered_by_manual_save");
    assert_eq!(consumed_by.as_deref(), Some(session_id));
    Ok(())
}

#[test]
fn summary_candidate_promotion_skips_high_similarity_recent_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-claim-near-match";
    let project = "test/proj";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Use session memory claims to suppress duplicate Stop summary candidates."
                .to_string(),
            title: Some("Near duplicate claim".to_string()),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    let claim_id = saved
        .claim_id
        .ok_or_else(|| anyhow::anyhow!("save_memory should return claim_id"))?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        Some("Use session memory claims to suppress duplicate summary candidates"),
        None,
        None,
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let noop_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_candidate_noops WHERE memory_claim_id = ?1",
        [claim_id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 0);
    assert_eq!(candidate_count, 0);
    assert_eq!(noop_count, 1);
    Ok(())
}

#[test]
fn summary_candidate_promotion_does_not_skip_different_session_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let project = "test/proj";
    let decision = "Session-specific claims must not suppress another session candidate";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: decision.to_string(),
            title: Some("Different session claim".to_string()),
            project: Some(project.to_string()),
            session_id: Some("session-a".to_string()),
            host: Some("codex-cli".to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    record_summary_evidence(&conn, "session-b", project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        "session-b",
        project,
        Some("Finish issue 287"),
        Some(decision),
        None,
        None,
    )?;

    let noop_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidate_noops", [], |row| {
            row.get(0)
        })?;
    let consumed_at: Option<i64> = conn.query_row(
        "SELECT consumed_at_epoch FROM memory_claims WHERE memory_id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 1);
    assert_eq!(noop_count, 0);
    assert_eq!(consumed_at, None);
    Ok(())
}

#[test]
fn summary_candidate_promotion_does_not_skip_different_host_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "shared-session-id";
    let project = "test/proj";
    let decision = "Host identity is part of exact session claim matching";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: decision.to_string(),
            title: Some("Different host claim".to_string()),
            project: Some(project.to_string()),
            session_id: Some(session_id.to_string()),
            host: Some("api".to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    record_summary_evidence_with_host(&conn, "codex-cli", session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        Some(decision),
        None,
        None,
    )?;

    let noop_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidate_noops", [], |row| {
            row.get(0)
        })?;
    let consumed_at: Option<i64> = conn.query_row(
        "SELECT consumed_at_epoch FROM memory_claims WHERE memory_id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 1);
    assert_eq!(noop_count, 0);
    assert_eq!(consumed_at, None);
    Ok(())
}

#[test]
fn summary_candidate_promotion_does_not_skip_different_owner_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-claim-owner-boundary";
    let project = "test/proj";
    let decision = "Claim matching respects memory owner boundaries";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: decision.to_string(),
            title: Some("Owner boundary claim".to_string()),
            project: Some(project.to_string()),
            session_id: Some(session_id.to_string()),
            host: Some("codex-cli".to_string()),
            memory_type: Some("decision".to_string()),
            scope: Some("global".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        Some(decision),
        None,
        None,
    )?;

    let noop_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidate_noops", [], |row| {
            row.get(0)
        })?;
    let consumed_at: Option<i64> = conn.query_row(
        "SELECT consumed_at_epoch FROM memory_claims WHERE memory_id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 1);
    assert_eq!(noop_count, 0);
    assert_eq!(consumed_at, None);
    Ok(())
}

#[test]
fn summary_candidate_promotion_does_not_skip_different_memory_type_claim() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-claim-type-boundary";
    let project = "test/proj";
    let fact = "Claim matching respects memory type boundaries";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: fact.to_string(),
            title: Some("Type boundary claim".to_string()),
            project: Some(project.to_string()),
            session_id: Some(session_id.to_string()),
            host: Some("codex-cli".to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        None,
        Some(fact),
        None,
    )?;

    let noop_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidate_noops", [], |row| {
            row.get(0)
        })?;
    let consumed_at: Option<i64> = conn.query_row(
        "SELECT consumed_at_epoch FROM memory_claims WHERE memory_id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 1);
    assert_eq!(noop_count, 0);
    assert_eq!(consumed_at, None);
    Ok(())
}

#[test]
fn summary_candidate_promotion_does_not_skip_low_similarity_candidate() -> Result<()> {
    let mut conn = setup_conn()?;
    let session_id = "session-claim-low-similarity";
    let project = "test/proj";
    let saved = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Use exact session memory claims only for identical saved facts".to_string(),
            title: Some("Exact claims".to_string()),
            project: Some(project.to_string()),
            session_id: Some(session_id.to_string()),
            memory_type: Some("decision".to_string()),
            local_copy_enabled: Some(false),
            ..SaveMemoryRequest::default()
        },
    )?;
    let claim_id = saved
        .claim_id
        .ok_or_else(|| anyhow::anyhow!("save_memory should return claim_id"))?;
    record_summary_evidence(&conn, session_id, project)?;

    let count = promote_summary_to_memory_candidates(
        &mut conn,
        session_id,
        project,
        Some("Finish issue 287"),
        Some("Switch summary extraction to keep unrelated facts in pending review"),
        None,
        None,
    )?;

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let noop_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidate_noops", [], |row| {
            row.get(0)
        })?;
    let consumed_at: Option<i64> = conn.query_row(
        "SELECT consumed_at_epoch FROM memory_claims WHERE id = ?1",
        [claim_id],
        |row| row.get(0),
    )?;

    assert_eq!(count, 1);
    assert_eq!(candidate_count, 1);
    assert_eq!(noop_count, 0);
    assert_eq!(consumed_at, None);
    Ok(())
}

#[test]
fn test_content_hash_dedup() {
    let hash1 = content_hash("Use FTS5 trigram tokenizer for CJK support");
    let hash2 = content_hash("Use FTS5 trigram tokenizer for CJK support");
    assert_eq!(hash1, hash2);

    let hash3 = content_hash("Switch to WAL mode for concurrent reads");
    assert_ne!(hash1, hash3);
}
