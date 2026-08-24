use super::*;
use anyhow::Context;

fn verified_candidates(conn: &Connection) -> Result<(ProcedureCandidate, ProcedureCandidate)> {
    let now = chrono::Utc::now().timestamp();
    let command = "cargo test";
    let workflow_key = crate::memory::slugify_for_topic(command, 64);
    let files = vec!["src/lib.rs".to_string()];
    let files_json = serde_json::to_string(&files)?;
    let mut traces = Vec::new();
    for seq in 1_i64..=3 {
        let captured = crate::db::record_captured_event(
            conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: "procedure-activation-receipt",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &serde_json::json!({
                    "seq": seq,
                    "event_type": "bash",
                    "exit_code": 0,
                    "tool_input": { "command": command },
                    "files": files,
                    "git_branch": "main"
                })
                .to_string(),
                task_kind: None,
            },
        )?;
        let (host_id, project_id, session_row_id, verified_at_epoch): (i64, i64, i64, i64) = conn
            .query_row(
            "SELECT host_id, project_id, session_row_id, created_at_epoch
             FROM captured_events WHERE id = ?1",
            [captured.event_row_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        conn.execute(
            "INSERT INTO procedure_verifications
             (host_id, project_id, session_row_id, branch, workflow_key, command,
              files_touched, source_event_id, verified_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, 'main', ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                host_id,
                project_id,
                session_row_id,
                workflow_key,
                command,
                files_json,
                captured.event_row_id,
                verified_at_epoch,
                now
            ],
        )?;
        traces.push(ProcedureTrace {
            project: "/tmp/remem".to_string(),
            branch: Some("main".to_string()),
            workflow_key: workflow_key.clone(),
            command: command.to_string(),
            files_touched: files.clone(),
            succeeded: true,
            verified_at_epoch,
            source_event_id: Some(captured.event_row_id),
        });
    }
    let initial =
        build_procedure_candidate(&traces[..2], now, &ProcedurePromotionPolicy::default())
            .context("two verified traces should build a procedure candidate")?;
    let expanded = build_procedure_candidate(&traces, now, &ProcedurePromotionPolicy::default())
        .context("three verified traces should build an expanded procedure candidate")?;
    Ok((initial, expanded))
}

#[test]
fn procedure_promotion_binds_verified_evidence_before_activation_receipt() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    let (candidate, expanded) = verified_candidates(&conn)?;

    let mut forged = candidate.clone();
    forged.source_event_ids.push(i64::MAX);
    let error = promote_procedure_memory(&conn, &forged)
        .expect_err("unverified procedure evidence must fail before activation");
    assert!(
        error.to_string().contains("verified evidence")
            || error.to_string().contains("evidence ids")
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        0
    );

    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (901, 'project', 'procedure', ?1, ?2, '[]', 0.8,
                 'low', 'approved', 1, 1)",
        params![candidate.topic_key, candidate.content],
    )?;
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          source_project, target_project, owner_scope, owner_key, context_class,
          source_trust_class, source_candidate_id)
         VALUES (?1, ?2, ?3, ?4, 'procedure', ?5, 1, 1, 1, 'active', 'main', 'project',
                 ?1, ?1, 'repo', ?1, 'startup_core', 'external_content', 901)",
        params![
            candidate.project,
            candidate.topic_key,
            candidate.title,
            candidate.content,
            serde_json::to_string(&candidate.files)?
        ],
    )?;

    let memory_id = promote_procedure_memory(&conn, &candidate)?;
    let replayed_id = promote_procedure_memory(&conn, &candidate)?;
    assert_eq!(replayed_id, memory_id);
    let (route_kind, result_sha256, receipt_count): (String, String, i64) = conn.query_row(
        "SELECT route_kind, result_sha256,
                (SELECT COUNT(*) FROM memory_activation_requests
                 WHERE result_memory_id = ?1)
         FROM memory_activation_requests
         WHERE result_memory_id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(route_kind, "candidate_promotion");
    assert_eq!(receipt_count, 1);
    let actual = crate::memory::activation::ExpectedActiveMemory::from_existing(&conn, memory_id)?;
    assert_eq!(actual.source_candidate_id, Some(901));
    assert_eq!(
        serde_json::from_str::<Vec<i64>>(
            actual
                .evidence_event_ids
                .as_deref()
                .context("procedure memory must retain evidence ids")?
        )?,
        candidate.source_event_ids
    );
    assert_eq!(result_sha256, actual.sha256());
    let retained_trust: String = conn.query_row(
        "SELECT source_trust_class FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(retained_trust, "external_content");

    let markdown_content = format!("{}\nMarkdown clarification.", candidate.content);
    let route = crate::memory::activation::load_existing_route(&conn, memory_id)?;
    let markdown_request = crate::memory::activation::ActiveMemoryWriteRequest {
        activation_id: "markdown:procedure-provenance-update".to_string(),
        route_kind: crate::memory::activation::ActivationRouteKind::BackupImport,
        actor_kind: crate::memory::activation::ActivationActorKind::Operator,
        source_operation: "markdown_update".to_string(),
        source_trust: crate::memory::poisoning::SourceTrustClass::RepoFile,
        result_source_trust: crate::memory::poisoning::SourceTrustClass::RepoFile,
        source_project: route.source_project,
        route: route.route,
        provenance_kind: crate::memory::activation::ActivationProvenanceKind::Backup,
        provenance_ref: "operator:markdown:procedure-provenance".to_string(),
        payload_sha256: crate::memory::activation::payload_sha256(&[
            "procedure-provenance-update",
            &markdown_content,
        ]),
        expected_memory: crate::memory::activation::ExpectedActiveMemory::new(
            &candidate.title,
            &markdown_content,
            "procedure",
        )
        .with_topic_key(Some(&candidate.topic_key))
        .with_files(Some(&serde_json::to_string(&candidate.files)?)),
        poisoning_verdict: crate::memory::activation::ActivationPoisoningVerdict::Clean,
        superseded_ids: Vec::new(),
    };
    crate::memory::activation::execute_one(&conn, &markdown_request, |_permit| {
        conn.execute(
            "UPDATE memories
             SET content = ?1, source_trust_class = 'repo_file',
                 source_candidate_id = NULL, evidence_event_ids = NULL
             WHERE id = ?2",
            params![markdown_content, memory_id],
        )?;
        Ok(memory_id)
    })?;
    conn.execute(
        "UPDATE memory_candidates
         SET topic_key = 'retagged-procedure', memory_type = 'discovery'
         WHERE id = 901",
        [],
    )?;
    assert_eq!(promote_procedure_memory(&conn, &candidate)?, memory_id);

    let replacement_id = promote_procedure_memory(&conn, &expanded)?;
    assert_ne!(replacement_id, memory_id);
    assert_eq!(
        conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            [memory_id],
            |row| { row.get::<_, String>(0) }
        )?,
        "stale"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        3
    );
    for id in [memory_id, replacement_id] {
        let stored_sha: String = conn.query_row(
            "SELECT result_sha256 FROM memory_activation_requests
             WHERE result_memory_id = ?1 ORDER BY rowid DESC LIMIT 1",
            [id],
            |row| row.get(0),
        )?;
        let row = crate::memory::activation::ExpectedActiveMemory::from_existing(&conn, id)?;
        assert_eq!(stored_sha, row.sha256());
    }
    assert_eq!(promote_procedure_memory(&conn, &candidate)?, memory_id);
    let replacement_trust: String = conn.query_row(
        "SELECT source_trust_class FROM memories WHERE id = ?1",
        [replacement_id],
        |row| row.get(0),
    )?;
    assert_eq!(replacement_trust, "repo_file");
    conn.execute(
        "UPDATE procedure_verifications SET verified_at_epoch = 1",
        [],
    )?;
    assert_eq!(promote_procedure_memory(&conn, &candidate)?, memory_id);
    Ok(())
}

#[test]
fn procedure_evidence_must_match_the_captured_success_event() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    let (candidate, _) = verified_candidates(&conn)?;
    conn.execute("UPDATE captured_events SET tool_name = 'Read'", [])?;

    assert!(super::evidence::load_verified_procedure_evidence(
        &conn,
        &candidate.source_event_ids,
        &candidate.project,
        &ProcedurePromotionPolicy::default(),
    )?
    .is_none());
    let error = promote_procedure_memory(&conn, &candidate)
        .expect_err("non-Bash captured events may not authenticate procedure evidence");
    assert!(error.to_string().contains("verified evidence"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn procedure_replay_uses_original_receipt_trust_after_agent_update() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    let (candidate, _) = verified_candidates(&conn)?;

    let memory_id = promote_procedure_memory(&conn, &candidate)?;
    let updated = crate::memory::service::save_memory_from_with_reference_time(
        &conn,
        &crate::memory::service::SaveMemoryRequest {
            text: format!("{}\nAgent clarification.", candidate.content),
            title: Some(candidate.title.clone()),
            project: Some(candidate.project.clone()),
            topic_key: Some(candidate.topic_key.clone()),
            memory_type: Some("procedure".to_string()),
            branch: candidate.branch.clone(),
            scope: Some("project".to_string()),
            local_copy_enabled: Some(false),
            claim_enabled: Some(false),
            idempotency_key: Some("procedure-agent-update".to_string()),
            ..crate::memory::service::SaveMemoryRequest::default()
        },
        None,
        crate::memory::service::SaveMemoryCaller::McpAgent,
    )?;
    assert_eq!(updated.id, memory_id);
    assert_eq!(
        conn.query_row(
            "SELECT source_trust_class FROM memories WHERE id = ?1",
            [memory_id],
            |row| row.get::<_, String>(0),
        )?,
        "external_content"
    );

    assert_eq!(promote_procedure_memory(&conn, &candidate)?, memory_id);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests WHERE result_memory_id = ?1",
            [memory_id],
            |row| row.get::<_, i64>(0),
        )?,
        2
    );
    Ok(())
}
