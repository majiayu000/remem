use rusqlite::params;

use crate::db::{record_captured_event, CaptureEventInput, ExtractionTaskKind};

use super::*;

fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

fn insert_pending_candidate(conn: &mut Connection, topic_key: &str, text: &str) -> Result<i64> {
    insert_pending_candidate_with_scope(conn, topic_key, text, "project")
}

fn insert_pending_candidate_with_scope(
    conn: &mut Connection,
    topic_key: &str,
    text: &str,
    scope: &str,
) -> Result<i64> {
    insert_pending_candidate_with_scope_and_type(conn, topic_key, text, scope, "decision")
}

fn insert_pending_candidate_with_scope_and_type(
    conn: &mut Connection,
    topic_key: &str,
    text: &str,
    scope: &str,
    memory_type: &str,
) -> Result<i64> {
    record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-review",
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: text,
            task_kind: Some(ExtractionTaskKind::MemoryCandidate),
        },
    )?;
    let task = crate::db::claim_next_extraction_task(conn, "worker-review", 60)?
        .expect("task should claim");
    let evidence_json = serde_json::to_string(&vec![task.high_watermark_event_id.unwrap()])?;
    // Release the lease so helpers can insert more than one candidate per test.
    crate::db::mark_extraction_task_done(
        conn,
        task.id,
        "worker-review",
        task.high_watermark_event_id,
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0.72, 'medium',
                 'pending_review', ?7, ?7)",
        params![
            task.project_id,
            scope,
            memory_type,
            topic_key,
            text,
            evidence_json,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn review_list_includes_evidence_preview() -> Result<()> {
    let mut conn = setup_conn();
    let id = insert_pending_candidate(&mut conn, "review-list", "Review this candidate")?;

    let rows = list_pending(&conn, None, 10)?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].project.as_deref(), Some("/tmp/remem"));
    assert!(rows[0].evidence_preview[0].contains("tool_result"));
    Ok(())
}

#[test]
fn review_approve_promotes_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let id = insert_pending_candidate(&mut conn, "review-approve", "Approve this memory")?;

    let memory_id = approve_candidate(&mut conn, id)?.expect("candidate should approve");

    let status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    let source_candidate_id: i64 = conn.query_row(
        "SELECT source_candidate_id FROM memories WHERE id = ?1",
        params![memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(status, "approved");
    assert_eq!(source_candidate_id, id);
    let (fact_predicate, fact_source_memory_id, fact_evidence): (String, i64, String) = conn
        .query_row(
            "SELECT predicate, source_memory_id, source_event_ids
             FROM memory_facts
             WHERE source_memory_id = ?1",
            params![memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    let candidate_evidence: String = conn.query_row(
        "SELECT evidence_event_ids FROM memory_candidates WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    assert_eq!(fact_predicate, "affects_project");
    assert_eq!(fact_source_memory_id, memory_id);
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&fact_evidence)?,
        serde_json::from_str::<Vec<i64>>(&candidate_evidence)?
    );
    Ok(())
}

#[test]
fn review_approve_lesson_candidate_creates_metadata() -> Result<()> {
    let mut conn = setup_conn();
    let id = insert_pending_candidate_with_scope_and_type(
        &mut conn,
        "review-lesson",
        "Lesson: generic lesson promotions must keep metadata so context can load them.",
        "project",
        "lesson",
    )?;

    let memory_id = approve_candidate(&mut conn, id)?.expect("candidate should approve");

    let (memory_type, metadata_count): (String, i64) = conn.query_row(
        "SELECT m.memory_type, COUNT(l.memory_id)
         FROM memories m
         LEFT JOIN memory_lessons l ON l.memory_id = m.id
         WHERE m.id = ?1",
        params![memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(memory_type, "lesson");
    assert_eq!(metadata_count, 1);
    Ok(())
}

#[test]
fn review_approve_lesson_candidate_supersedes_old_lesson() -> Result<()> {
    let mut conn = setup_conn();
    let old_id = crate::memory::lesson::save_lesson(
        &conn,
        &crate::memory::lesson::SaveLessonRequest {
            session_id: None,
            project: "/tmp/remem",
            topic_key: Some("review-lesson-update"),
            title: "Old lesson",
            content: "Old lesson content",
            confidence: 0.8,
            source_evidence: None,
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )?;
    let id = insert_pending_candidate_with_scope_and_type(
        &mut conn,
        "review-lesson-update",
        "Updated lesson content",
        "project",
        "lesson",
    )?;

    let new_id = approve_candidate(&mut conn, id)?.expect("candidate should approve");

    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![old_id],
        |row| row.get(0),
    )?;
    let (content, metadata_count): (String, i64) = conn.query_row(
        "SELECT m.content, COUNT(l.memory_id)
         FROM memories m
         LEFT JOIN memory_lessons l ON l.memory_id = m.id
         WHERE m.id = ?1",
        params![new_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(old_status, "stale");
    assert_eq!(content, "Updated lesson content");
    assert_eq!(metadata_count, 1);
    assert_eq!(memory_count, 2);
    Ok(())
}

#[test]
fn review_discard_marks_candidate_without_deleting_evidence() -> Result<()> {
    let mut conn = setup_conn();
    let id = insert_pending_candidate(&mut conn, "review-discard", "Discard this memory")?;

    assert!(discard_candidate(&conn, id)?);

    let (status, evidence): (String, String) = conn.query_row(
        "SELECT review_status, evidence_event_ids FROM memory_candidates WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "discarded");
    assert!(evidence.contains('1'));
    Ok(())
}

#[test]
fn review_edit_promotes_edited_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let id = insert_pending_candidate(&mut conn, "review-edit", "Original memory")?;

    let memory_id = edit_candidate(
        &mut conn,
        id,
        CandidateEdit {
            topic_key: Some("edited-topic".to_string()),
            memory_type: Some("architecture".to_string()),
            text: Some("Edited architecture memory".to_string()),
            ..CandidateEdit::default()
        },
    )?
    .expect("candidate should edit");

    let (status, topic_key, memory_type, text): (String, String, String, String) = conn.query_row(
        "SELECT c.review_status, m.topic_key, m.memory_type, m.content
             FROM memory_candidates c
             JOIN memories m ON m.id = ?2
             WHERE c.id = ?1",
        params![id, memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(status, "edited");
    assert_eq!(topic_key, "edited-topic");
    assert_eq!(memory_type, "architecture");
    assert_eq!(text, "Edited architecture memory");
    Ok(())
}

#[test]
fn review_invalid_ids_are_reported() -> Result<()> {
    let mut conn = setup_conn();

    assert!(approve_candidate(&mut conn, 999)?.is_none());
    assert!(!discard_candidate(&conn, 999)?);
    assert!(edit_candidate(
        &mut conn,
        999,
        CandidateEdit {
            text: Some("missing".to_string()),
            ..CandidateEdit::default()
        },
    )?
    .is_none());
    Ok(())
}

#[test]
fn review_approve_rejects_already_promoted_candidate_without_duplicate_memory() -> Result<()> {
    let mut conn = setup_conn();
    let id = insert_pending_candidate(&mut conn, "review-no-duplicate", "Approve once")?;

    let memory_id = approve_candidate(&mut conn, id)?
        .ok_or_else(|| anyhow::anyhow!("candidate should approve"))?;
    let err = match approve_candidate(&mut conn, id) {
        Ok(_) => anyhow::bail!("second approve should fail"),
        Err(err) => err,
    };

    assert!(err.to_string().contains(&format!(
        "candidate {id} is approved, expected pending_review"
    )));
    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE source_candidate_id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    let source_candidate_id: i64 = conn.query_row(
        "SELECT source_candidate_id FROM memories WHERE id = ?1",
        params![memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 1);
    assert_eq!(source_candidate_id, id);
    Ok(())
}

#[test]
fn review_approve_supersedes_duplicate_topic_memory() -> Result<()> {
    let mut conn = setup_conn();
    let old_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("review-dup"),
        "Existing",
        "Existing memory",
        "decision",
        None,
        None,
        "project",
        None,
    )?;
    let id = insert_pending_candidate(&mut conn, "review-dup", "Updated memory")?;

    approve_candidate(&mut conn, id)?.expect("candidate should approve");

    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    let (content, owner_scope, owner_key): (String, String, String) = conn.query_row(
        "SELECT content, owner_scope, owner_key FROM memories
         WHERE topic_key = 'review-dup' AND status = 'active'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![old_id],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 2);
    assert_eq!(content, "Updated memory");
    assert_eq!(old_status, "stale");
    assert_eq!(owner_scope, "repo");
    assert_eq!(owner_key, "/tmp/remem");
    Ok(())
}

#[test]
fn batch_preview_and_mutation_resolve_the_same_id_set() -> Result<()> {
    let mut conn = setup_conn();
    let a = insert_pending_candidate(&mut conn, "batch-parity-a", "First candidate")?;
    let b = insert_pending_candidate(&mut conn, "batch-parity-b", "Second candidate")?;

    let filter = BatchFilter {
        limit: BATCH_LIMIT_DEFAULT,
        ..BatchFilter::default()
    };
    let preview = resolve_batch(&conn, &filter)?;
    let meta = ReviewMeta::batch("tester", "batch-parity", None);
    let outcome = approve_batch(&mut conn, &preview, &meta)?;

    assert_eq!(preview.ids, vec![a, b]);
    assert_eq!(outcome.processed, preview.ids);
    assert_eq!(outcome.promoted_memory_ids.len(), 2);
    Ok(())
}

#[test]
fn batch_approve_failure_rolls_back_all_rows() -> Result<()> {
    let mut conn = setup_conn();
    let ok_id = insert_pending_candidate(&mut conn, "batch-tx-ok", "Valid candidate")?;
    // A pending candidate with no project anywhere makes promote_row fail
    // mid-batch.
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (NULL, 'project', 'decision', 'batch-tx-bad', 'Broken', '[]',
                 0.7, 'medium', 'pending_review', ?1, ?1)",
        params![now + 10],
    )?;

    let filter = BatchFilter {
        limit: BATCH_LIMIT_DEFAULT,
        ..BatchFilter::default()
    };
    let preview = resolve_batch(&conn, &filter)?;
    let meta = ReviewMeta::batch("tester", "batch-tx", None);
    assert!(approve_batch(&mut conn, &preview, &meta).is_err());

    let (status, actor): (String, Option<String>) = conn.query_row(
        "SELECT review_status, review_actor FROM memory_candidates WHERE id = ?1",
        params![ok_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE source_candidate_id IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(actor, None);
    assert_eq!(memory_count, 0);
    Ok(())
}

#[test]
fn batch_filter_limit_is_enforced() -> Result<()> {
    let mut conn = setup_conn();
    insert_pending_candidate(&mut conn, "batch-cap-a", "One")?;
    insert_pending_candidate(&mut conn, "batch-cap-b", "Two")?;
    insert_pending_candidate(&mut conn, "batch-cap-c", "Three")?;

    let capped = resolve_batch(
        &conn,
        &BatchFilter {
            limit: 2,
            ..BatchFilter::default()
        },
    )?;
    let defaulted = resolve_batch(
        &conn,
        &BatchFilter {
            limit: 0,
            ..BatchFilter::default()
        },
    )?;

    assert_eq!(capped.ids.len(), 2);
    assert_eq!(defaulted.ids.len(), 3);
    Ok(())
}

#[test]
fn review_actions_persist_actor_metadata() -> Result<()> {
    let mut conn = setup_conn();
    let single = insert_pending_candidate(&mut conn, "meta-single", "Single approve")?;
    let batch = insert_pending_candidate(&mut conn, "meta-batch", "Batch discard")?;

    approve_candidate_with_meta(&mut conn, single, &ReviewMeta::single("alice"))?
        .expect("candidate should approve");
    let meta = ReviewMeta::batch("bob", "batch-42", Some("obsolete".to_string()));
    let filter = BatchFilter {
        topic_key: Some("meta-batch".to_string()),
        limit: BATCH_LIMIT_DEFAULT,
        ..BatchFilter::default()
    };
    let preview = resolve_batch(&conn, &filter)?;
    discard_batch(&mut conn, &preview, &meta)?;

    let (actor, source, batch_id, reviewed_at): (String, String, Option<String>, i64) = conn
        .query_row(
            "SELECT review_actor, review_action_source, review_batch_id, reviewed_at_epoch
             FROM memory_candidates WHERE id = ?1",
            params![single],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    assert_eq!(actor, "alice");
    assert_eq!(source, "single");
    assert_eq!(batch_id, None);
    assert!(reviewed_at > 0);

    let (actor, source, batch_id, reason): (String, String, String, String) = conn.query_row(
        "SELECT review_actor, review_action_source, review_batch_id, review_reason
         FROM memory_candidates WHERE id = ?1",
        params![batch],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(actor, "bob");
    assert_eq!(source, "batch");
    assert_eq!(batch_id, "batch-42");
    assert_eq!(reason, "obsolete");
    Ok(())
}

#[test]
fn batch_filters_select_matching_rows_only() -> Result<()> {
    let mut conn = setup_conn();
    let matching = insert_pending_candidate(&mut conn, "filter-match", "Contains needle text")?;
    insert_pending_candidate(&mut conn, "filter-other", "Different content")?;
    conn.execute(
        "UPDATE memory_candidates SET auto_promote_block_reason = 'risk_class_not_low'
         WHERE id = ?1",
        params![matching],
    )?;

    let by_reason = resolve_batch(
        &conn,
        &BatchFilter {
            block_reason: Some("risk_class_not_low".to_string()),
            limit: BATCH_LIMIT_DEFAULT,
            ..BatchFilter::default()
        },
    )?;
    let by_contains = resolve_batch(
        &conn,
        &BatchFilter {
            contains: Some("needle".to_string()),
            limit: BATCH_LIMIT_DEFAULT,
            ..BatchFilter::default()
        },
    )?;

    assert_eq!(by_reason.ids, vec![matching]);
    assert_eq!(by_contains.ids, vec![matching]);
    Ok(())
}

#[test]
fn batch_mutation_uses_previewed_ids_not_new_matches() -> Result<()> {
    let mut conn = setup_conn();
    let previewed = insert_pending_candidate(&mut conn, "batch-stable-a", "Original match")?;
    let filter = BatchFilter {
        contains: Some("match".to_string()),
        limit: BATCH_LIMIT_DEFAULT,
        ..BatchFilter::default()
    };
    let preview = resolve_batch(&conn, &filter)?;
    let late = insert_pending_candidate(&mut conn, "batch-stable-b", "Late match")?;

    let outcome = approve_batch(
        &mut conn,
        &preview,
        &ReviewMeta::batch("tester", "batch-stable", None),
    )?;

    assert_eq!(preview.ids, vec![previewed]);
    assert_eq!(outcome.processed, vec![previewed]);
    let late_status: String = conn.query_row(
        "SELECT review_status FROM memory_candidates WHERE id = ?1",
        params![late],
        |row| row.get(0),
    )?;
    assert_eq!(late_status, "pending_review");
    Ok(())
}

#[test]
fn batch_filter_project_matches_routed_candidates() -> Result<()> {
    let mut conn = setup_conn();
    let routed = insert_pending_candidate(&mut conn, "batch-routed", "Routed candidate")?;
    conn.execute(
        "UPDATE memory_candidates
         SET source_project = '/repo/source', target_project = '/repo/target',
             owner_scope = 'repo', owner_key = '/repo/target'
         WHERE id = ?1",
        params![routed],
    )?;

    let preview = resolve_batch(
        &conn,
        &BatchFilter {
            project: Some("/repo/target".to_string()),
            limit: BATCH_LIMIT_DEFAULT,
            ..BatchFilter::default()
        },
    )?;

    assert_eq!(preview.ids, vec![routed]);
    assert_eq!(preview.by_project, vec![("/repo/target".to_string(), 1)]);
    Ok(())
}

#[test]
fn batch_contains_filter_treats_like_wildcards_literally() -> Result<()> {
    let mut conn = setup_conn();
    let percent = insert_pending_candidate(&mut conn, "batch-percent", "Contains 100% literal")?;
    let underscore = insert_pending_candidate(
        &mut conn,
        "batch-underscore",
        "Contains under_score literal",
    )?;
    insert_pending_candidate(&mut conn, "batch-normal", "Contains ordinary text")?;

    let by_percent = resolve_batch(
        &conn,
        &BatchFilter {
            contains: Some("%".to_string()),
            limit: BATCH_LIMIT_DEFAULT,
            ..BatchFilter::default()
        },
    )?;
    let by_underscore = resolve_batch(
        &conn,
        &BatchFilter {
            contains: Some("_".to_string()),
            limit: BATCH_LIMIT_DEFAULT,
            ..BatchFilter::default()
        },
    )?;

    assert_eq!(by_percent.ids, vec![percent]);
    assert_eq!(by_underscore.ids, vec![underscore]);
    Ok(())
}

#[test]
fn batch_filter_rejects_negative_older_than() -> Result<()> {
    let conn = setup_conn();
    let err = resolve_batch(
        &conn,
        &BatchFilter {
            older_than_days: Some(-1),
            limit: BATCH_LIMIT_DEFAULT,
            ..BatchFilter::default()
        },
    )
    .expect_err("negative older_than_days should fail");
    assert!(err.to_string().contains("non-negative"));
    Ok(())
}

#[test]
fn review_approve_preserves_existing_project_memory_for_global_candidate() -> Result<()> {
    let mut conn = setup_conn();
    crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("review-scope"),
        "Project",
        "Project memory",
        "decision",
        None,
        None,
        "project",
        None,
    )?;
    let id =
        insert_pending_candidate_with_scope(&mut conn, "review-scope", "Global memory", "global")?;

    approve_candidate(&mut conn, id)?.expect("candidate should approve");

    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE topic_key = 'review-scope'",
        [],
        |row| row.get(0),
    )?;
    let project_content: String = conn.query_row(
        "SELECT content FROM memories
         WHERE topic_key = 'review-scope' AND scope = 'project'",
        [],
        |row| row.get(0),
    )?;
    let global_content: String = conn.query_row(
        "SELECT content FROM memories
         WHERE topic_key = 'review-scope' AND scope = 'global'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 2);
    assert_eq!(project_content, "Project memory");
    assert_eq!(global_content, "Global memory");
    Ok(())
}
