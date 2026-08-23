use rusqlite::params;

use super::*;

fn request(activation_id: &str, payload: &str) -> ActiveMemoryWriteRequest {
    ActiveMemoryWriteRequest {
        activation_id: activation_id.to_string(),
        route_kind: ActivationRouteKind::RustApi,
        actor_kind: ActivationActorKind::RustApi,
        source_operation: "save_memory".to_string(),
        source_trust: SourceTrustClass::LocalToolOutput,
        result_source_trust: SourceTrustClass::LocalToolOutput,
        source_project: "/repo".to_string(),
        route: ActiveMemoryRoute::default_for("/repo", None, "project"),
        provenance_kind: ActivationProvenanceKind::RustApi,
        provenance_ref: "rust-api:test".to_string(),
        payload_sha256: payload_sha256(&[payload]),
        expected_memory: ExpectedActiveMemory::new("title", payload, "discovery"),
        poisoning_verdict: ActivationPoisoningVerdict::Clean,
        superseded_ids: Vec::new(),
    }
}

fn insert_memory(conn: &Connection, content: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO memories
         (project, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, target_project,
          owner_scope, owner_key, context_class, source_trust_class)
         VALUES ('/repo', 'title', ?1, 'discovery', 1, 1, 'active',
                 'project', '/repo', '/repo', 'repo', '/repo',
                     'startup_core', 'local_tool_output')",
        [content],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn identical_activation_replays_without_running_writer() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let request = request("save:stable", "same");
    let first = execute_one(&conn, &request, |_| insert_memory(&conn, "same"))?;
    let replay = execute_one(&conn, &request, |_| bail!("writer must not replay"))?;
    assert_eq!(first.memory_id, replay.memory_id);
    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    Ok(())
}

#[test]
fn replay_rejects_partial_acknowledgement_metadata_for_clean_receipt() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let request = request("save:partial-replay-ack", "same");
    let first = execute_one(&conn, &request, |_| insert_memory(&conn, "same"))?;
    conn.execute(
        "UPDATE memories
         SET acknowledged_pattern_id = 'override_previous_instructions',
             acknowledged_pattern_version = 1,
             acknowledged_at_epoch = NULL
         WHERE id = ?1",
        [first.memory_id],
    )?;

    let error = execute_one(&conn, &request, |_| bail!("writer must not replay"))
        .expect_err("partial acknowledgement evidence must fail closed");

    assert!(error.to_string().contains("incomplete acknowledgement"));
    Ok(())
}

#[test]
fn migrated_v086_supplemental_receipt_replays_with_legacy_fingerprint() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let mut original_request = request("save:v086-supplemental", "legacy");
    original_request.route_kind = ActivationRouteKind::SupplementalSave;
    original_request.provenance_kind = ActivationProvenanceKind::SupplementalSave;
    original_request.provenance_ref = "rust-api:test".to_string();
    let memory_id = insert_memory(&conn, "legacy")?;
    let request_sha256 = super::receipt::v086_request_sha256(&original_request, &[])?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
          local_copy_status, created_at_epoch)
         VALUES (?1, ?2, 'supplemental_save', 'rust_api', 'save_memory',
                 'local_tool_output', 'legacy_v086_source_local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'supplemental_save',
                 'rust-api:test', ?3, ?4, 'clean', '[]', ?5, 'disabled',
                 'disabled', 1)",
        params![
            original_request.activation_id,
            request_sha256,
            original_request.payload_sha256,
            original_request.expected_memory.sha256(),
            memory_id,
        ],
    )?;

    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids, confidence, risk_class,
          review_status, created_at_epoch, updated_at_epoch, source_trust_class)
         VALUES (73, 'project', 'discovery', 'legacy-topic', 'legacy', '[501]', 0.95, 'low',
                 'approved', 2, 2, 'user_prompt')",
        [],
    )?;
    let candidate_expected = original_request
        .expected_memory
        .clone()
        .with_candidate_evidence(Some("[501]"), Some(73));
    let mut promotion = request("candidate:replacement-73", "legacy");
    promotion.route_kind = ActivationRouteKind::CandidatePromotion;
    promotion.actor_kind = ActivationActorKind::Operator;
    promotion.source_operation = "candidate_review".to_string();
    promotion.source_trust = SourceTrustClass::UserPrompt;
    promotion.result_source_trust = SourceTrustClass::UserPrompt;
    promotion.provenance_kind = ActivationProvenanceKind::Candidate;
    promotion.provenance_ref = "candidate:73".to_string();
    promotion.expected_memory = candidate_expected.clone();
    promotion.poisoning_verdict = ActivationPoisoningVerdict::UpstreamValidated;
    promotion.superseded_ids = vec![memory_id];
    execute_one(&conn, &promotion, |_| {
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            [memory_id],
        )?;
        conn.execute(
            "INSERT INTO memories
             (project, title, content, memory_type, evidence_event_ids,
              source_candidate_id, created_at_epoch, updated_at_epoch, status,
              scope, source_project, target_project, owner_scope, owner_key,
              context_class, source_trust_class)
             VALUES ('/repo', 'title', 'legacy', 'discovery', '[501]', 73, 2, 2,
                     'active', 'project', '/repo', '/repo', 'repo', '/repo',
                     'startup_core', 'user_prompt')",
            [],
        )?;
        Ok(conn.last_insert_rowid())
    })?;

    let mut retry = original_request.clone();
    retry.expected_memory = candidate_expected;
    retry.result_source_trust = SourceTrustClass::UserPrompt;
    let replay = execute_supplemental_save(&conn, &retry, |_| bail!("writer must not replay"))?;
    assert_eq!(replay.memory_id, memory_id);
    assert!(replay.replayed);
    assert_eq!(
        replay.supplemental_receipt,
        Some(SupplementalSaveReceipt::Disabled)
    );
    Ok(())
}

#[test]
fn replay_survives_a_later_governed_in_place_update() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let first_request = request("save:first", "first");
    let first = execute_one(&conn, &first_request, |_| insert_memory(&conn, "first"))?;
    let second_request = request("save:second", "second");
    let second = execute_one(&conn, &second_request, |_| {
        conn.execute(
            "UPDATE memories SET content = 'second' WHERE id = ?1",
            [first.memory_id],
        )?;
        Ok(first.memory_id)
    })?;
    assert_eq!(second.memory_id, first.memory_id);

    let replay = execute_one(&conn, &first_request, |_| bail!("writer must not replay"))?;
    assert_eq!(replay.memory_id, first.memory_id);
    assert!(replay.replayed);
    Ok(())
}

#[test]
fn legacy_dream_receipt_replays_after_same_row_provenance_drift() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let mut original = request("dream:legacy-provenance", "legacy-dream");
    original.route_kind = ActivationRouteKind::DreamConsolidation;
    original.actor_kind = ActivationActorKind::AutomaticWorker;
    original.source_operation = "dream_consolidation".to_string();
    original.source_trust = SourceTrustClass::ExternalContent;
    original.result_source_trust = SourceTrustClass::ExternalContent;
    original.provenance_kind = ActivationProvenanceKind::Generated;
    original.provenance_ref = "dream-generated:legacy-provenance".to_string();
    original.poisoning_verdict = ActivationPoisoningVerdict::UpstreamValidated;
    original.expected_memory = original
        .expected_memory
        .clone()
        .with_candidate_evidence(Some("[701]"), None);
    let memory_id = conn.query_row(
        "INSERT INTO memories
         (project, title, content, memory_type, evidence_event_ids,
          created_at_epoch, updated_at_epoch, status, scope, source_project,
          target_project, owner_scope, owner_key, context_class, source_trust_class)
         VALUES ('/repo', 'title', 'legacy-dream', 'discovery', '[701]',
                 1, 1, 'active', 'project', '/repo', '/repo', 'repo', '/repo',
                 'startup_core', 'external_content')
         RETURNING id",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let legacy_request_sha256 = super::receipt::v086_request_sha256(&original, &[])?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, created_at_epoch)
         VALUES (?1, ?2, 'dream_consolidation', 'automatic_worker', 'dream_consolidation',
                 'external_content', 'legacy_v086_source_external_content', '/repo', '/repo',
                 0, NULL, 'project', 'repo', '/repo', '/repo', 'generated', ?3,
                 ?4, ?5, 'upstream_validated', '[]', ?6, 1)",
        params![
            original.activation_id,
            legacy_request_sha256,
            original.provenance_ref,
            original.payload_sha256,
            original.expected_memory.sha256(),
            memory_id,
        ],
    )?;

    let later = request("save:later-provenance-clear", "later");
    execute_one(&conn, &later, |_| {
        conn.execute(
            "UPDATE memories
             SET content = 'later', evidence_event_ids = NULL,
                 source_trust_class = 'local_tool_output'
             WHERE id = ?1",
            [memory_id],
        )?;
        Ok(memory_id)
    })?;
    let mut retry = original.clone();
    retry.expected_memory.evidence_event_ids = None;
    let replay = replay_dream_if_present(&conn, &retry)?.expect("legacy Dream replay");
    assert!(replay.replayed);
    assert_eq!(replay.memory_id, memory_id);
    Ok(())
}

#[test]
fn earlier_v086_receipt_replays_after_a_later_v086_same_row_update() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let first_request = request("save:v086-first", "first");
    let memory_id = insert_memory(&conn, "first")?;
    let request_sha256 = super::receipt::v086_request_sha256(&first_request, &[])?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, created_at_epoch)
         VALUES (?1, ?2, 'rust_api', 'rust_api', 'save_memory',
                 'local_tool_output', 'legacy_v086_source_local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'rust_api',
                 'rust-api:test', ?3, ?4, 'clean', '[]', ?5, 1)",
        params![
            first_request.activation_id,
            request_sha256,
            first_request.payload_sha256,
            first_request.expected_memory.sha256(),
            memory_id,
        ],
    )?;

    let second_request = request("save:v086-second", "second");
    let second_request_sha256 = super::receipt::v086_request_sha256(&second_request, &[])?;
    conn.execute(
        "UPDATE memories SET content = 'second' WHERE id = ?1",
        [memory_id],
    )?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, created_at_epoch)
         VALUES (?1, ?2, 'rust_api', 'rust_api', 'save_memory',
                 'local_tool_output', 'legacy_v086_source_local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'rust_api',
                 'rust-api:test', ?3, ?4, 'clean', '[]', ?5, 2)",
        params![
            second_request.activation_id,
            second_request_sha256,
            second_request.payload_sha256,
            second_request.expected_memory.sha256(),
            memory_id,
        ],
    )?;

    let replay = execute_one(&conn, &first_request, |_| bail!("writer must not replay"))?;
    assert_eq!(replay.memory_id, memory_id);
    assert!(replay.replayed);
    Ok(())
}

#[test]
fn migrated_v086_supplemental_replay_ignores_later_in_place_result_provenance() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let mut first_request = request("save:v086-supplemental-in-place", "first");
    first_request.route_kind = ActivationRouteKind::SupplementalSave;
    first_request.provenance_kind = ActivationProvenanceKind::SupplementalSave;
    let memory_id = insert_memory(&conn, "first")?;
    let request_sha256 = super::receipt::v086_request_sha256(&first_request, &[])?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
          local_copy_status, created_at_epoch)
         VALUES (?1, ?2, 'supplemental_save', 'rust_api', 'save_memory',
                 'local_tool_output', 'legacy_v086_source_local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'supplemental_save',
                 'rust-api:test', ?3, ?4, 'clean', '[]', ?5, 'disabled',
                 'disabled', 1)",
        params![
            first_request.activation_id,
            request_sha256,
            first_request.payload_sha256,
            first_request.expected_memory.sha256(),
            memory_id,
        ],
    )?;

    let second_request = request("save:v087-same-row", "second");
    execute_one(&conn, &second_request, |_| {
        conn.execute(
            "UPDATE memories SET content = 'second' WHERE id = ?1",
            [memory_id],
        )?;
        Ok(memory_id)
    })?;

    let mut retry = first_request.clone();
    retry.expected_memory = ExpectedActiveMemory::new("title", "second", "discovery");
    let mut changed_retry = retry.clone();
    changed_retry.payload_sha256 = payload_sha256(&["changed caller payload"]);
    let error = execute_supplemental_save(&conn, &changed_retry, |_| {
        bail!("writer must not run for a conflicting retry")
    })
    .expect_err("changed caller payload must not match the immutable receipt");
    assert!(error.to_string().contains("reused with different request"));
    let replay = execute_supplemental_save(&conn, &retry, |_| bail!("writer must not replay"))?;
    assert_eq!(replay.memory_id, memory_id);
    assert!(replay.replayed);
    assert_eq!(
        replay.supplemental_receipt,
        Some(SupplementalSaveReceipt::Disabled)
    );
    Ok(())
}

#[test]
fn replay_rejects_route_drift_after_a_later_activation() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let first_request = request("save:first-route", "first");
    let first = execute_one(&conn, &first_request, |_| insert_memory(&conn, "first"))?;
    let second_request = request("save:second-route", "second");
    execute_one(&conn, &second_request, |_| {
        conn.execute(
            "UPDATE memories SET content = 'second' WHERE id = ?1",
            [first.memory_id],
        )?;
        Ok(first.memory_id)
    })?;
    conn.execute(
        "UPDATE memories SET owner_key = '/tampered' WHERE id = ?1",
        [first.memory_id],
    )?;

    let error = execute_one(&conn, &first_request, |_| bail!("writer must not replay"))
        .expect_err("route drift after a later activation must fail replay");
    assert!(error.to_string().contains("owner key has drifted"));
    conn.execute(
        "UPDATE memories
         SET owner_key = '/repo', source_trust_class = 'external_content'
         WHERE id = ?1",
        [first.memory_id],
    )?;
    let error = execute_one(&conn, &first_request, |_| bail!("writer must not replay"))
        .expect_err("result trust drift after a later activation must fail replay");
    assert!(error.to_string().contains("result trust has drifted"));
    Ok(())
}

#[test]
fn replay_rejects_archive_after_a_later_reactivation() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let original_request = request("save:original", "original");
    let original = execute_one(&conn, &original_request, |_| {
        insert_memory(&conn, "original")
    })?;

    let mut replacement_request = request("save:replacement", "replacement");
    replacement_request.superseded_ids = vec![original.memory_id];
    let replacement = execute_one(&conn, &replacement_request, |_| {
        let replacement_id = insert_memory(&conn, "replacement")?;
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            [original.memory_id],
        )?;
        Ok(replacement_id)
    })?;

    let mut reactivation_request = request("save:reactivation", "original");
    reactivation_request.superseded_ids = vec![replacement.memory_id];
    execute_one(&conn, &reactivation_request, |_| {
        conn.execute(
            "UPDATE memories SET status = 'active' WHERE id = ?1",
            [original.memory_id],
        )?;
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            [replacement.memory_id],
        )?;
        Ok(original.memory_id)
    })?;
    conn.execute(
        "UPDATE memories SET status = 'archived' WHERE id = ?1",
        [original.memory_id],
    )?;

    let error = execute_one(&conn, &reactivation_request, |_| {
        bail!("writer must not replay")
    })
    .expect_err("an older supersede receipt must not authorize an archived replay");
    assert!(error
        .to_string()
        .contains("inactive without a superseding receipt"));
    Ok(())
}

#[test]
fn supplemental_save_cannot_use_generic_activation_without_receipt() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let mut request = request("save:missing-receipt", "same");
    request.route_kind = ActivationRouteKind::SupplementalSave;
    request.actor_kind = ActivationActorKind::Agent;
    request.source_trust = SourceTrustClass::ExternalContent;
    request.result_source_trust = SourceTrustClass::ExternalContent;
    request.provenance_kind = ActivationProvenanceKind::SupplementalSave;
    request.provenance_ref = "mcp:agent".to_string();

    let error = execute_one(&conn, &request, |_| bail!("writer must not run"))
        .expect_err("a supplemental activation without a receipt must fail");

    assert!(error
        .to_string()
        .contains("must use the durable receipt path"));
    Ok(())
}

#[test]
fn changed_payload_cannot_reuse_activation_id() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    execute_one(&conn, &request("save:stable", "first"), |_| {
        insert_memory(&conn, "first")
    })?;
    let err = execute_one(&conn, &request("save:stable", "changed"), |_| {
        insert_memory(&conn, "changed")
    })
    .expect_err("changed payload must fail");
    assert!(err.to_string().contains("reused with different request"));
    Ok(())
}

#[test]
fn route_mismatch_rolls_back_memory_and_ledger() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let err = execute_one(&conn, &request("save:wrong-route", "wrong"), |_| {
        conn.execute(
            "INSERT INTO memories
             (project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, scope, source_project, target_project,
              owner_scope, owner_key, context_class, source_trust_class)
             VALUES ('/other', 'title', 'wrong', 'discovery', 1, 1,
                     'active', 'project', '/other', '/other', 'repo',
                     '/other', 'startup_core', 'external_content')",
            [],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .expect_err("wrong route must fail");
    assert!(err.to_string().contains("postcondition"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn malformed_route_and_agent_trust_fail_before_writer() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let mut invalid = request("save:invalid", "invalid");
    invalid.route.branch = Some("  ".to_string());
    invalid.source_trust = SourceTrustClass::UserPrompt;
    let error = execute_one(&conn, &invalid, |_| bail!("writer must not run"))
        .expect_err("malformed route/trust must fail");
    assert!(error.to_string().contains("branch must"));
    Ok(())
}

#[test]
fn undeclared_supersede_rolls_back_the_entire_activation() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let first = insert_memory(&conn, "first")?;
    let second = insert_memory(&conn, "second")?;
    let mut activation = request("save:delta", "replacement");
    activation.superseded_ids = vec![first];
    let error = execute_one(&conn, &activation, |_| {
        let replacement = insert_memory(&conn, "replacement")?;
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id IN (?1, ?2)",
            params![first, second],
        )?;
        Ok(replacement)
    })
    .expect_err("undeclared supersede must fail");
    assert!(error.to_string().contains("active-set removal drift"));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        2
    );
    Ok(())
}
