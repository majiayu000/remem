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
