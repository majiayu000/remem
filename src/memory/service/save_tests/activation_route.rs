use super::*;

#[test]
fn direct_save_topic_match_ignores_newer_row_outside_repo_owner_route() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-topic-owner-route");
    let conn = db::open_db()?;
    let request = SaveMemoryRequest {
        text: "Initial repo-owned content.".to_string(),
        title: Some("Repo target".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("owner-route-target".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let repo_memory = save_memory(&conn, &request)?;
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, owner_scope,
          owner_key, context_class, source_trust_class)
         VALUES ('proj', 'owner-route-target', 'Tool-owned collision',
                 'Must remain untouched.', 'discovery', 1, 9999999999,
                 'active', 'project', 'proj', 'tool', 'tool:example',
                 'startup_core', 'local_tool_output')",
        [],
    )?;
    let tool_memory = conn.last_insert_rowid();
    let updated = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Updated repo-owned content.".to_string(),
            title: Some("Repo target updated".to_string()),
            ..request
        },
    )?;

    assert_eq!(updated.id, repo_memory.id);
    let tool_content: String = conn.query_row(
        "SELECT content FROM memories WHERE id = ?1",
        [tool_memory],
        |row| row.get(0),
    )?;
    assert_eq!(tool_content, "Must remain untouched.");
    Ok(())
}

#[test]
fn agent_direct_save_update_preserves_candidate_provenance_in_receipt() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-update-candidate-provenance");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_trust_class)
         VALUES (41, 'project', 'discovery', 'captured-target',
                 'Automatically captured fact.', '[101,102]', 0.9, 'low',
                 'approved', 1, 1, 'user_prompt')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, evidence_event_ids,
          source_candidate_id, created_at_epoch, updated_at_epoch, status, scope,
          source_project, target_project, owner_scope, owner_key, context_class,
          source_trust_class)
         VALUES ('proj', 'captured-target', 'Captured target',
                 'Automatically captured fact.', 'discovery', '[101,102]', 41,
                 1, 1, 'active', 'project', 'proj', 'proj', 'repo', 'proj',
                 'startup_core', 'user_prompt')",
        [],
    )?;
    let memory_id = conn.last_insert_rowid();

    let saved = crate::memory::service::save_memory_from_with_reference_time(
        &conn,
        &SaveMemoryRequest {
            text: "Automatically captured fact with an agent-supplied clarification.".to_string(),
            title: Some("Captured target clarified".to_string()),
            project: Some("proj".to_string()),
            topic_key: Some("captured-target".to_string()),
            memory_type: Some("discovery".to_string()),
            scope: Some("project".to_string()),
            local_copy_enabled: Some(false),
            claim_enabled: Some(false),
            idempotency_key: Some("candidate-provenance-update".to_string()),
            ..SaveMemoryRequest::default()
        },
        None,
        crate::memory::service::SaveMemoryCaller::McpAgent,
    )?;

    assert_eq!(saved.id, memory_id);
    assert_eq!(saved.operation, "update");
    let (evidence, candidate_id): (String, Option<i64>) = conn.query_row(
        "SELECT evidence_event_ids, source_candidate_id FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(evidence, "[101,102]");
    assert_eq!(candidate_id, Some(41));
    let stored_result_sha256: String = conn.query_row(
        "SELECT result_sha256 FROM memory_activation_requests WHERE result_memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    let actual = crate::memory::activation::ExpectedActiveMemory::from_existing(&conn, memory_id)?;
    assert_eq!(stored_result_sha256, actual.sha256());
    Ok(())
}

#[test]
fn direct_save_replays_original_receipt_after_equivalent_candidate_replacement(
) -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-replay-candidate-replacement");
    let conn = db::open_db()?;
    let request = SaveMemoryRequest {
        text: "Stable represented fact.".to_string(),
        title: Some("Stable fact".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("stable-replay-target".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        claim_enabled: Some(false),
        idempotency_key: Some("stable-replay-key".to_string()),
        ..SaveMemoryRequest::default()
    };
    let original = save_memory(&conn, &request)?;
    let conflict = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Changed caller payload.".to_string(),
            ..request.clone()
        },
    )
    .expect_err("same key with changed caller payload must conflict");
    assert!(conflict
        .to_string()
        .contains("reused with different request"));

    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_trust_class)
         VALUES (73, 'project', 'discovery', 'stable-replay-target',
                 'Stable represented fact.', '[501]', 0.95, 'low', 'approved',
                 2, 2, 'user_prompt')",
        [],
    )?;
    let expected = crate::memory::activation::ExpectedActiveMemory::new(
        "Stable fact",
        "Stable represented fact.",
        "discovery",
    )
    .with_topic_key(Some("stable-replay-target"))
    .with_candidate_evidence(Some("[501]"), Some(73));
    let promotion = crate::memory::activation::ActiveMemoryWriteRequest {
        activation_id: "candidate:replacement-73".to_string(),
        route_kind: crate::memory::activation::ActivationRouteKind::CandidatePromotion,
        actor_kind: crate::memory::activation::ActivationActorKind::Operator,
        source_operation: "candidate_review".to_string(),
        source_trust: crate::memory::poisoning::SourceTrustClass::UserPrompt,
        result_source_trust: crate::memory::poisoning::SourceTrustClass::UserPrompt,
        source_project: "proj".to_string(),
        route: crate::memory::activation::ActiveMemoryRoute::default_for("proj", None, "project"),
        provenance_kind: crate::memory::activation::ActivationProvenanceKind::Candidate,
        provenance_ref: "candidate:73".to_string(),
        payload_sha256: crate::memory::activation::payload_sha256(&[
            "candidate:73",
            "Stable represented fact.",
        ]),
        expected_memory: expected,
        poisoning_verdict: crate::memory::activation::ActivationPoisoningVerdict::UpstreamValidated,
        superseded_ids: vec![original.id],
    };
    let replacement = crate::memory::activation::execute_one(&conn, &promotion, |_permit| {
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            [original.id],
        )?;
        conn.execute(
            "INSERT INTO memories
             (project, topic_key, title, content, memory_type, evidence_event_ids,
              source_candidate_id, created_at_epoch, updated_at_epoch, status, scope,
              source_project, target_project, owner_scope, owner_key, context_class,
              source_trust_class)
             VALUES ('proj', 'stable-replay-target', 'Stable fact',
                     'Stable represented fact.', 'discovery', '[501]', 73, 2, 2,
                     'active', 'project', 'proj', 'proj', 'repo', 'proj',
                     'startup_core', 'user_prompt')",
            [],
        )?;
        Ok(conn.last_insert_rowid())
    })?;
    let replay = save_memory(&conn, &request)?;
    assert_eq!(replay.id, original.id);
    assert_eq!(replay.operation, "noop");
    assert_eq!(replay.claim_status, "disabled");
    let statuses: (String, String) = conn.query_row(
        "SELECT old.status, replacement.status
         FROM memories AS old, memories AS replacement
         WHERE old.id = ?1 AND replacement.id = ?2",
        rusqlite::params![original.id, replacement.memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(statuses, ("stale".to_string(), "active".to_string()));
    Ok(())
}
