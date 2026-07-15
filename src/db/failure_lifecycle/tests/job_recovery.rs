use super::*;

#[test]
fn failure_lifecycle_dream_recovery_merges_different_profile_into_pending_canonical() -> Result<()>
{
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let source = insert_failed_job(
        &conn,
        "incoming-host",
        JobType::Dream,
        "/dream-profile-merge",
        None,
        Some("retry incoming Dream profile"),
        now - 1_000,
        "transient",
        0,
    )?;
    let incoming_payload = r#"{"remem_ai_profile":"quality"}"#;
    conn.execute(
        "UPDATE jobs SET payload_json = ?1, priority = 25 WHERE id = ?2",
        params![incoming_payload, source],
    )?;
    let canonical = crate::db::maybe_enqueue_dream_job(
        &conn,
        "canonical-host",
        "/dream-profile-merge",
        r#"{"remem_ai_profile":"balanced"}"#,
        80,
        60,
    )?
    .job_id();

    let outcome = recover_due_job_candidate(&conn, source, now)?;

    assert_eq!(
        outcome,
        Some(JobRecoveryOutcome::Coalesced {
            source_id: source,
            canonical_id: canonical,
            identity_kind: crate::db::JobIdentityKind::Dream,
        })
    );
    let canonical_snapshot: (String, String, i64, String) = conn.query_row(
        "SELECT host, payload_json, priority, state FROM jobs WHERE id = ?1",
        params![canonical],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        canonical_snapshot,
        (
            "incoming-host".to_string(),
            incoming_payload.to_string(),
            25,
            "pending".to_string(),
        )
    );
    Ok(())
}

#[test]
fn failure_lifecycle_dream_recovery_does_not_mutate_processing_canonical() -> Result<()> {
    let conn = setup_conn()?;
    let now = chrono::Utc::now().timestamp();
    let source = insert_failed_job(
        &conn,
        "incoming-host",
        JobType::Dream,
        "/dream-processing-immutable",
        None,
        Some("retry incoming Dream profile"),
        now - 1_000,
        "transient",
        0,
    )?;
    conn.execute(
        "UPDATE jobs SET payload_json = ?1, priority = 25 WHERE id = ?2",
        params![r#"{"remem_ai_profile":"quality"}"#, source],
    )?;
    let canonical_payload = r#"{"remem_ai_profile":"balanced"}"#;
    let canonical = crate::db::maybe_enqueue_dream_job(
        &conn,
        "canonical-host",
        "/dream-processing-immutable",
        canonical_payload,
        80,
        60,
    )?
    .job_id();
    conn.execute(
        "UPDATE jobs SET state = 'processing', lease_owner = 'worker-a',
         lease_expires_epoch = ?1 WHERE id = ?2",
        params![now + 60, canonical],
    )?;
    let canonical_before = job_snapshot(&conn, canonical)?;

    let outcome = recover_due_job_candidate(&conn, source, now)?;

    assert!(matches!(
        outcome,
        Some(JobRecoveryOutcome::Coalesced {
            canonical_id,
            identity_kind: crate::db::JobIdentityKind::Dream,
            ..
        }) if canonical_id == canonical
    ));
    assert_eq!(job_snapshot(&conn, canonical)?, canonical_before);
    Ok(())
}

#[test]
fn failure_lifecycle_dream_recovery_keeps_pending_canonical_for_same_or_empty_profile() -> Result<()>
{
    for (suffix, incoming_payload) in [
        ("same", r#"{"remem_ai_profile":"balanced"}"#),
        ("empty", r#"{"remem_ai_profile":"  "}"#),
    ] {
        let conn = setup_conn()?;
        let now = chrono::Utc::now().timestamp();
        let project = format!("/dream-{suffix}-profile");
        let source = insert_failed_job(
            &conn,
            "incoming-host",
            JobType::Dream,
            &project,
            None,
            Some("retry incoming Dream profile"),
            now - 1_000,
            "transient",
            0,
        )?;
        conn.execute(
            "UPDATE jobs SET payload_json = ?1, priority = 25 WHERE id = ?2",
            params![incoming_payload, source],
        )?;
        let canonical = crate::db::maybe_enqueue_dream_job(
            &conn,
            "canonical-host",
            &project,
            r#"{"remem_ai_profile":"balanced"}"#,
            80,
            60,
        )?
        .job_id();
        let canonical_before = job_snapshot(&conn, canonical)?;

        assert!(matches!(
            recover_due_job_candidate(&conn, source, now)?,
            Some(JobRecoveryOutcome::Coalesced { canonical_id, .. })
                if canonical_id == canonical
        ));
        assert_eq!(
            job_snapshot(&conn, canonical)?,
            canonical_before,
            "{suffix}"
        );
    }
    Ok(())
}
