use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db;

use super::tests::{setup_conn, setup_task};
use super::{process_with_generator, MemoryCandidateResult};

fn insert_source_observation_typed(
    conn: &Connection,
    task: &db::ExtractionTask,
    observation_type: &str,
    text: &str,
) -> Result<()> {
    let obs_id = db::insert_observation_with_branch(
        conn,
        "capture-observation-test",
        &task.project,
        observation_type,
        Some("Typed observation"),
        None,
        Some(text),
        None,
        None,
        None,
        None,
        None,
        12,
        None,
        None,
    )?;
    let event_id = task.high_watermark_event_id.unwrap_or(1);
    conn.execute(
        "UPDATE observations
         SET host_id = ?1,
             project_id = ?2,
             session_row_id = ?3,
             observation_type = ?4,
             text = ?5,
             evidence_event_ids = ?6,
             confidence = ?7
         WHERE id = ?8",
        params![
            task.host_id,
            task.project_id,
            task.session_row_id,
            observation_type,
            text,
            serde_json::to_string(&vec![event_id])?,
            0.91,
            obs_id
        ],
    )?;
    Ok(())
}

#[tokio::test]
async fn memory_candidate_auto_promotes_architecture_from_discovery_observation() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-architecture")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "discovery",
        "The extraction worker uses a single-writer SQLite connection per process.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>architecture</type><topic_key>architecture-worker-db</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>The extraction worker uses a single-writer SQLite connection per process.</text></memory_candidate>".to_string())
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
    let (memory_type, review_status): (String, String) = conn.query_row(
        "SELECT memory_type, review_status FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(memory_type, "architecture");
    assert_eq!(review_status, "auto_promoted");
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(memory_count, 1);
    let embedding_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE m.topic_key = 'architecture-worker-db'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(embedding_count, 1);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_auto_promotes_discovery_from_feature_observation() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-feature")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "Added a structured retrieval gate that scores candidates before promotion.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-retrieval-gate</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>Added a structured retrieval gate that scores candidates before promotion.</text></memory_candidate>".to_string())
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
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "auto_promoted");
    Ok(())
}

#[tokio::test]
async fn memory_candidate_auto_promotes_condensed_candidate_from_token_overlap() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-condensed-overlap")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The worker lifecycle now records auto promote block reasons in memory candidates, keeping pending review diagnostics visible for operator status screens.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-auto-promote-block-reasons</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>Worker lifecycle records auto promote block reasons for memory candidate diagnostics.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0,
            to_event_id: task
                .high_watermark_event_id
                .ok_or_else(|| anyhow::anyhow!("task watermark"))?
        }
    );
    let (review_status, block_reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(review_status, "auto_promoted");
    assert_eq!(block_reason, None);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_keeps_weak_overlap_pending_with_block_reason() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-weak-overlap")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The review dashboard shows memory candidate counts and pending review diagnostics.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-auto-promote-support</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>Auto promotion support validates rewritten durable memory facts against source observations.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1,
            to_event_id: task
                .high_watermark_event_id
                .ok_or_else(|| anyhow::anyhow!("task watermark"))?
        }
    );
    let (review_status, block_reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(
        block_reason.as_deref(),
        Some("no_supporting_source_observation")
    );
    Ok(())
}

#[tokio::test]
async fn memory_candidate_keeps_inverted_action_overlap_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-inverted-action")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The worker lifecycle disables auto promote block reasons for memory candidate diagnostics.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-auto-promote-support</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>Worker lifecycle records auto promote block reasons for memory candidate diagnostics.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1,
            to_event_id: task
                .high_watermark_event_id
                .ok_or_else(|| anyhow::anyhow!("task watermark"))?
        }
    );
    let (review_status, block_reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(
        block_reason.as_deref(),
        Some("no_supporting_source_observation")
    );
    Ok(())
}

#[tokio::test]
async fn memory_candidate_keeps_architecture_unsupported_by_bugfix_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-architecture-bugfix")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "bugfix",
        "The extraction worker uses a single-writer SQLite connection per process.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>architecture</type><topic_key>architecture-worker-db</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>The extraction worker uses a single-writer SQLite connection per process.</text></memory_candidate>".to_string())
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
    let (review_status, block_reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(
        block_reason.as_deref(),
        Some("no_supporting_source_observation")
    );
    Ok(())
}
