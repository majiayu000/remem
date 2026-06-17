use anyhow::Result;
use rusqlite::params;

use crate::memory_candidate::review::approve_candidate;

use super::{
    insert_source_observation, low_risk_candidate_xml, process_with_generator, setup_conn,
    setup_task, MemoryCandidateResult,
};

#[tokio::test]
async fn memory_candidate_existing_same_topic_same_text_becomes_noop() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-noop")?;
    let text = "Use the worker loop to process extraction tasks after observation extraction.";
    insert_source_observation(&conn, &task, text)?;
    crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("decision-worker-loop"),
        "Existing worker loop",
        text,
        "decision",
        None,
        None,
        "project",
        None,
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 0,
            to_event_id: task.high_watermark_event_id.expect("task watermark")
        }
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(review_status, "noop");
    assert_eq!(memory_count, 1);
    let (operation, source_candidate_id, noop_reason): (String, Option<i64>, Option<String>) = conn
        .query_row(
            "SELECT operation, source_candidate_id, noop_reason
             FROM memory_operation_log
             ORDER BY id DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    let candidate_id: i64 =
        conn.query_row("SELECT id FROM memory_candidates", [], |row| row.get(0))?;
    assert_eq!(operation, "noop");
    assert_eq!(source_candidate_id, Some(candidate_id));
    assert_eq!(
        noop_reason.as_deref(),
        Some("already represented by active memory")
    );
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_edges", [], |row| row.get(0))?;
    assert_eq!(edge_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_newer_same_topic_supersedes_old_memory() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-update")?;
    let text =
        "Use the async worker loop to process extraction tasks after observation extraction.";
    insert_source_observation(&conn, &task, text)?;
    let old_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("decision-worker-loop"),
        "Old worker loop",
        "Use the synchronous worker loop to process extraction tasks after observation extraction.",
        "decision",
        None,
        None,
        "project",
        None,
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>decision-worker-loop</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.92</confidence>\
                <text>{text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0,
            to_event_id: task.high_watermark_event_id.expect("task watermark")
        }
    );
    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![old_id],
        |row| row.get(0),
    )?;
    let (new_id, active_text, candidate_id, candidate_evidence): (i64, String, i64, String) = conn
        .query_row(
            "SELECT m.id, m.content, c.id, c.evidence_event_ids
             FROM memories m
             JOIN memory_candidates c ON c.id = m.source_candidate_id
             WHERE m.topic_key = 'decision-worker-loop' AND m.status = 'active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(old_status, "stale");
    assert_eq!(active_text, text);
    assert_eq!(memory_count, 2);
    let (operation_id, operation, superseded_ids): (i64, String, String) = conn.query_row(
        "SELECT id, operation, superseded_ids
         FROM memory_operation_log
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(operation, "update");
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&superseded_ids)?,
        vec![old_id]
    );

    let supersedes_edge: (String, i64, i64, Option<i64>, String, Option<i64>) = conn.query_row(
        "SELECT edge_type, from_memory_id, to_memory_id, source_candidate_id,
                evidence_event_ids, source_operation_id
         FROM memory_edges
         WHERE edge_type = 'supersedes'
           AND from_memory_id = ?1
           AND to_memory_id = ?2",
        params![old_id, new_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    assert_eq!(supersedes_edge.0, "supersedes");
    assert_eq!(supersedes_edge.1, old_id);
    assert_eq!(supersedes_edge.2, new_id);
    assert_eq!(supersedes_edge.3, Some(candidate_id));
    assert_eq!(supersedes_edge.5, Some(operation_id));
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&supersedes_edge.4)?,
        serde_json::from_str::<Vec<i64>>(&candidate_evidence)?
    );
    let derived_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_edges
         WHERE edge_type = 'derived_from'
           AND from_memory_id IS NULL
           AND to_memory_id = ?1
           AND source_candidate_id = ?2
           AND source_operation_id = ?3",
        params![new_id, candidate_id, operation_id],
        |row| row.get(0),
    )?;
    assert_eq!(derived_count, 1);
    Ok(())
}

#[tokio::test]
async fn preference_candidate_approval_consolidates_generic_paraphrase() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-preference-generic-update")?;
    insert_source_observation(
        &conn,
        &task,
        "User prefers brief Chinese status notes during long-running work.",
    )?;
    let old_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("preference-11111111"),
        "Progress update preference",
        "Prefer concise Chinese progress updates.",
        "preference",
        None,
        None,
        "project",
        None,
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate>\
                <scope>project</scope>\
                <type>preference</type>\
                <topic_key>preference-22222222</topic_key>\
                <risk_class>medium</risk_class>\
                <confidence>0.91</confidence>\
                <text>Prefer brief Chinese status notes.</text>\
             </memory_candidate>"
            .to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1,
            to_event_id: task.high_watermark_event_id.expect("task watermark")
        }
    );
    let candidate_id: i64 =
        conn.query_row("SELECT id FROM memory_candidates", [], |row| row.get(0))?;
    let new_id = approve_candidate(&mut conn, candidate_id)?.expect("candidate should approve");

    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![old_id],
        |row| row.get(0),
    )?;
    let (new_status, new_content): (String, String) = conn.query_row(
        "SELECT status, content FROM memories WHERE id = ?1",
        params![new_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(old_status, "stale");
    assert_eq!(new_status, "active");
    assert_eq!(new_content, "Prefer brief Chinese status notes.");
    let (operation, superseded_ids, reason): (String, String, String) = conn.query_row(
        "SELECT operation, superseded_ids, reason
         FROM memory_operation_log
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(operation, "update");
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&superseded_ids)?,
        vec![old_id]
    );
    assert!(reason.contains("generic preference consolidation kind=refinement"));
    let supersedes_edge_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_edges
         WHERE edge_type = 'supersedes'
           AND from_memory_id = ?1
           AND to_memory_id = ?2",
        params![old_id, new_id],
        |row| row.get(0),
    )?;
    assert_eq!(supersedes_edge_count, 1);
    Ok(())
}

#[tokio::test]
async fn preference_candidate_approval_records_generic_conflict() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-preference-generic-conflict")?;
    insert_source_observation(
        &conn,
        &task,
        "User said not to provide brief Chinese status notes.",
    )?;
    let old_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("preference-11111111"),
        "Progress update preference",
        "Prefer concise Chinese progress updates.",
        "preference",
        None,
        None,
        "project",
        None,
    )?;

    process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate>\
                <scope>project</scope>\
                <type>preference</type>\
                <topic_key>preference-22222222</topic_key>\
                <risk_class>medium</risk_class>\
                <confidence>0.91</confidence>\
                <text>Do not provide brief Chinese status notes.</text>\
             </memory_candidate>"
            .to_string())
    })
    .await?;
    let candidate_id: i64 =
        conn.query_row("SELECT id FROM memory_candidates", [], |row| row.get(0))?;
    let new_id = approve_candidate(&mut conn, candidate_id)?.expect("candidate should approve");

    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![old_id],
        |row| row.get(0),
    )?;
    let new_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![new_id],
        |row| row.get(0),
    )?;
    assert_eq!(old_status, "active");
    assert_eq!(new_status, "active");
    let (operation, conflicting_ids, reason): (String, String, String) = conn.query_row(
        "SELECT operation, conflicting_ids, reason
         FROM memory_operation_log
         ORDER BY id DESC
         LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(operation, "conflict");
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(&conflicting_ids)?,
        vec![old_id]
    );
    assert!(reason.contains("generic preference consolidation kind=contradiction"));
    let conflict_edge_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_edges
         WHERE edge_type = 'conflicts'
           AND from_memory_id = ?1
           AND to_memory_id = ?2",
        params![old_id, new_id],
        |row| row.get(0),
    )?;
    assert_eq!(conflict_edge_count, 1);
    Ok(())
}
