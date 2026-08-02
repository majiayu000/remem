use anyhow::{anyhow, Result};

use super::tests::{setup_conn, setup_task};
use super::tests_autopromote::insert_source_observation_typed;
use super::{process_with_generator, MemoryCandidateResult};

#[tokio::test]
async fn auto_promotes_supported_negative_fact() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-supported-negative-fact")?;
    let claim = "The cleanup task does not delete active memory rows after a retry failure.";
    insert_source_observation_typed(&conn, &task, "bugfix", claim)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "<memory_candidate><scope>project</scope><type>bugfix</type><topic_key>bugfix-cleanup-preserves-active-rows</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>{claim}</text></memory_candidate>"
        ))
    })
    .await?;

    assert_promoted(&task, result)
}

#[tokio::test]
async fn auto_promotes_non_secret_token_terminology() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-token-terminology")?;
    let claim = "The context compiler enforces a 4096 token budget for injected memory text.";
    insert_source_observation_typed(&conn, &task, "feature", claim)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-context-token-budget</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>{claim}</text></memory_candidate>"
        ))
    })
    .await?;

    assert_promoted(&task, result)
}

#[tokio::test]
async fn auto_promotes_claims_supported_across_observations() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-claim-level-support")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The extraction worker records promotion block reasons in candidate rows.",
    )?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The status command reports promotion block reason counts to operators.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-promotion-audit-reasons</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>The extraction worker records promotion block reasons in candidate rows. The status command reports promotion block reason counts to operators.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_promoted(&task, result)
}

#[tokio::test]
async fn keeps_partially_unsupported_multi_claim_candidate_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-partial-claim-support")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The extraction worker records promotion block reasons in candidate rows.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-partial-support</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>The extraction worker records promotion block reasons in candidate rows. The status command automatically repairs every blocked candidate.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_pending(&conn, result, "no_supporting_source_observation")
}

#[tokio::test]
async fn keeps_failed_action_from_supporting_positive_claim() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-failed-action-polarity")?;
    insert_source_observation_typed(
        &conn,
        &task,
        "feature",
        "The extraction worker fails to record durable promotion audit rows in the database.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-audit-row-recording</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>The extraction worker records durable promotion audit rows in the database.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_pending(&conn, result, "no_supporting_source_observation")
}

#[tokio::test]
async fn auto_promotes_supported_failure_lesson() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-failure-lesson")?;
    let claim = "After a failed migration replay, reopen the database before retrying the worker.";
    insert_source_observation_typed(&conn, &task, "bugfix", claim)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "<memory_candidate><scope>project</scope><type>lesson</type><topic_key>lesson-migration-replay-recovery</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>{claim}</text></memory_candidate>"
        ))
    })
    .await?;

    assert_promoted(&task, result)
}

#[tokio::test]
async fn keeps_non_failure_lesson_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-non-failure-lesson")?;
    let claim = "Group related status counters together in operator output.";
    insert_source_observation_typed(&conn, &task, "decision", claim)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "<memory_candidate><scope>project</scope><type>lesson</type><topic_key>lesson-status-counter-layout</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>{claim}</text></memory_candidate>"
        ))
    })
    .await?;

    assert_pending(&conn, result, "lesson_not_failure_qualified")
}

#[tokio::test]
async fn keeps_preferences_and_procedures_fail_closed() -> Result<()> {
    for memory_type in ["preference", "procedure"] {
        let mut conn = setup_conn();
        let task = setup_task(&mut conn, &format!("sess-candidate-{memory_type}-closed"))?;
        let claim = "Run targeted tests before the full project suite.";
        insert_source_observation_typed(&conn, &task, "decision", claim)?;
        let response = format!(
            "<memory_candidate><scope>project</scope><type>{memory_type}</type><topic_key>{memory_type}-targeted-tests</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>{claim}</text></memory_candidate>"
        );

        let result =
            process_with_generator(&mut conn, &task, |_prompt| async { Ok(response) }).await?;
        assert_pending(&conn, result, "memory_type_not_auto_promotable")?;
    }
    Ok(())
}

#[tokio::test]
async fn keeps_exact_access_token_claim_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-access-token-secret")?;
    let claim = "The deployment access token appears in the local worker error report.";
    insert_source_observation_typed(&conn, &task, "feature", claim)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "<memory_candidate><scope>project</scope><type>discovery</type><topic_key>discovery-access-token-report</topic_key><risk_class>low</risk_class><confidence>0.92</confidence><text>{claim}</text></memory_candidate>"
        ))
    })
    .await?;

    assert_pending(&conn, result, "contains_unsafe_marker")
}

fn assert_promoted(task: &crate::db::ExtractionTask, result: MemoryCandidateResult) -> Result<()> {
    let to_event_id = task
        .high_watermark_event_id
        .ok_or_else(|| anyhow!("task watermark"))?;
    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0,
            to_event_id,
        }
    );
    Ok(())
}

fn assert_pending(
    conn: &rusqlite::Connection,
    result: MemoryCandidateResult,
    expected_reason: &str,
) -> Result<()> {
    assert!(matches!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1,
            ..
        }
    ));
    let (status, reason): (String, Option<String>) = conn.query_row(
        "SELECT review_status, auto_promote_block_reason FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "pending_review");
    assert_eq!(reason.as_deref(), Some(expected_reason));
    Ok(())
}
