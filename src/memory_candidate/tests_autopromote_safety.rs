use anyhow::Result;

use super::tests::{setup_conn, setup_task};
use super::tests_autopromote::insert_source_observation_typed;
use super::{process_with_generator, MemoryCandidateResult};

#[tokio::test]
async fn memory_candidate_keeps_failure_pass_polarity_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-failure-pass-polarity")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "Source evidence validation failures for condensed memory candidate promotion are visible in diagnostics.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-validation-passes</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>Source evidence validation passes for condensed memory candidate promotion.</text></memory_candidate>".to_string())
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
async fn memory_candidate_requires_support_for_security_modifier() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-security-modifier")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "API gateway stores JWT auth keys inside the Redis cache layer for operator login flows.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-encrypted-jwt-cache</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>API gateway stores encrypted JWT auth keys in Redis cache layer.</text></memory_candidate>".to_string())
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
async fn memory_candidate_auto_promotes_supported_security_modifier() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-supported-security-modifier")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The API gateway stores encrypted JWT auth keys inside the Redis cache layer for operator login flows.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-encrypted-jwt-cache-supported</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>API gateway stores encrypted JWT auth key in Redis cache layer.</text></memory_candidate>".to_string())
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
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "auto_promoted");
    Ok(())
}

#[tokio::test]
async fn memory_candidate_keeps_future_tense_observation_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-future-tense")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The ingestion worker will persist durable memory candidates from summarized observations.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-future-tense</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>Ingestion worker persists durable memory candidates from summarized observations.</text></memory_candidate>".to_string())
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
