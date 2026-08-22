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
