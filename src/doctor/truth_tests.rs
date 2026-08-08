use super::*;

fn test_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn insert_memory(
    conn: &Connection,
    id: i64,
    topic: &str,
    status: &str,
    updated_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status)
         VALUES (?1, '/repo', ?2, 'Decision', ?2, 'decision', 1, ?3, ?4)",
        params![id, topic, updated_at, status],
    )?;
    Ok(())
}

fn options() -> TruthDoctorOptions {
    TruthDoctorOptions {
        project: "/repo".to_string(),
        branch: None,
        as_of_epoch: Some(100),
        subject: None,
        json: true,
        quiet: false,
    }
}

fn seed_graph_provenance(conn: &Connection) -> Result<(i64, i64, i64)> {
    let host_id: i64 =
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT INTO workspaces
         (root_path, created_at_epoch, updated_at_epoch)
         VALUES ('/doctor-truth-workspace', 1, 1)",
        [],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects
         (workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/repo', 'doctor-truth', 1, 1)",
        [workspace_id],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions
         (host_id, workspace_id, project_id, session_id, last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, 'doctor-truth-session', 1, 'active')",
        params![host_id, workspace_id, project_id],
    )?;
    let session_row_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captured_events
         (host_id, workspace_id, project_id, session_row_id, session_id, event_id,
          event_type, content_hash, retention_class, created_at_epoch, inserted_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'doctor-truth-session', 'doctor-truth-event',
                 'message', 'doctor-truth-hash', 'default', 1, 1)",
        params![host_id, workspace_id, project_id, session_row_id],
    )?;
    let event_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'deploy', 'deploy', ?2,
                 0.9, 'low', 'accepted', 1, 1)",
        params![project_id, format!("[{event_id}]")],
    )?;
    let candidate_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_operation_log
         (operation, planner_version, actor, source, source_candidate_id,
          result_memory_id, confidence, reason, created_at_epoch)
         VALUES ('update', 'doctor-truth-test', 'test', 'memory_candidate',
                 ?1, 2, 0.9, 'test provenance', 1)",
        [candidate_id],
    )?;
    let operation_id = conn.last_insert_rowid();
    Ok((event_id, candidate_id, operation_id))
}

#[test]
fn report_surfaces_conflicts_supersedes_and_noncurrent_references() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "superseded", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', 1, 2, 20)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('conflicts', 1, 2, 21)",
        [],
    )?;
    let before = conn.total_changes();

    let report = build_truth_report(&conn, &options())?;

    assert_eq!(
        conn.total_changes(),
        before,
        "diagnostic must be SELECT-only"
    );
    assert_eq!(report.counts.truth_items, 1);
    assert_eq!(report.counts.current, 1);
    assert_eq!(report.counts.supersedes_relations, 1);
    assert_eq!(report.counts.reference_issues, 1);
    assert!(report.reference_issues.iter().any(|issue| {
        issue.claim_ref == "memory:1" && issue.stored_status.as_deref() == Some("superseded")
    }));
    assert!(report.lifecycle_mappings.iter().any(|mapping| {
        mapping.object_kind == "memory"
            && mapping.stored_status == "superseded"
            && mapping.validity == ValidityState::Superseded
    }));
    Ok(())
}

#[test]
fn unresolved_equal_claims_warn_without_claim_text() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "runtime", "active", 20)?;
    insert_memory(&conn, 2, "runtime", "active", 20)?;

    let report = build_truth_report(&conn, &options())?;
    let encoded = serde_json::to_string(&report)?;

    assert_eq!(report.status, "warn");
    assert_eq!(truth_outcome(&report).exit_code(), 1);
    assert_eq!(report.counts.contradicted, 1);
    assert_eq!(report.conflicts[0].claim_refs, ["memory:1", "memory:2"]);
    assert!(!encoded.contains("Decision: runtime"));
    Ok(())
}

#[test]
fn as_of_excludes_future_and_not_yet_valid_relations() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "superseded", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', 1, 2, 20), ('supersedes', 1, 2, 101)",
        [],
    )?;
    let (event_id, candidate_id, operation_id) = seed_graph_provenance(&conn)?;
    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind,
          to_node_id, source_event_ids, source_candidate_id, source_operation_id,
          confidence, reason, valid_from_epoch, created_at_epoch)
         VALUES ('supersedes', 'trusted', 'memory', 1, 'memory', 2,
                 ?1, ?2, ?3, 0.9, 'future relation', 101, 90)",
        params![format!("[{event_id}]"), candidate_id, operation_id],
    )?;

    let report = build_truth_report(&conn, &options())?;

    assert_eq!(report.counts.supersedes_relations, 1);
    assert!(report
        .supersedes
        .iter()
        .all(|link| link.relation_ref.starts_with("memory_edge:")));
    assert!(!report
        .lifecycle_mappings
        .iter()
        .any(|mapping| mapping.object_kind == "trusted_graph_relation"));
    Ok(())
}

#[test]
fn graph_supersedes_uses_current_to_old_direction() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "stale", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    let (event_id, candidate_id, operation_id) = seed_graph_provenance(&conn)?;
    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind,
          to_node_id, source_event_ids, source_candidate_id, source_operation_id,
          confidence, reason, created_at_epoch)
         VALUES ('supersedes', 'trusted', 'memory', 2, 'memory', 1,
                 ?1, ?2, ?3, 0.9, 'current replaces old', 20)",
        params![format!("[{event_id}]"), candidate_id, operation_id],
    )?;

    let report = build_truth_report(&conn, &options())?;

    assert_eq!(report.status, "ok");
    assert_eq!(report.counts.current, 1);
    assert_eq!(report.counts.abstentions, 0);
    assert!(report.reference_issues.is_empty());
    assert_eq!(report.supersedes.len(), 1);
    assert_eq!(report.supersedes[0].newer_claim_ref, "memory:2");
    assert_eq!(report.supersedes[0].older_claim_ref, "memory:1");
    Ok(())
}

#[test]
fn lifecycle_mappings_exclude_objects_created_after_as_of() -> Result<()> {
    let conn = test_conn()?;
    conn.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status)
         VALUES (1, '/repo', 'future', 'Future', 'future', 'decision',
                 101, 101, 'archived')",
        [],
    )?;
    let (event_id, _, _) = seed_graph_provenance(&conn)?;
    let (project_id, session_row_id): (i64, i64) = conn.query_row(
        "SELECT project_id, session_row_id FROM captured_events WHERE id = ?1",
        [event_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    conn.execute(
        "INSERT INTO observations
         (memory_session_id, project, type, title, status, session_row_id,
          created_at_epoch)
         VALUES ('doctor-truth-session', '/repo', 'decision', 'Future',
                 'compressed', ?1, 101)",
        [session_row_id],
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'future', 'future', '[]',
                 0.9, 'low', 'noop', 101, 101)",
        [project_id],
    )?;
    conn.execute(
        "INSERT INTO user_context_claims
         (owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status,
          created_at_epoch, updated_at_epoch)
         VALUES ('repo', '/repo', 'preference', 'future', 'future', 1.0,
                 'normal', 'manual', '[]', 'pending_review', 101, 101)",
        [],
    )?;

    let report = build_truth_report(&conn, &options())?;

    for (object_kind, status) in [
        ("memory", "archived"),
        ("observation", "compressed"),
        ("memory_candidate", "noop"),
        ("user_context_claim", "pending_review"),
    ] {
        assert!(!report.lifecycle_mappings.iter().any(|mapping| {
            mapping.object_kind == object_kind && mapping.stored_status == status
        }));
    }
    Ok(())
}

#[test]
fn cross_project_graph_relation_is_visible_when_it_affects_projection() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "active", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    conn.execute("UPDATE memories SET project = '/other' WHERE id = 2", [])?;
    let (event_id, candidate_id, operation_id) = seed_graph_provenance(&conn)?;
    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind,
          to_node_id, source_event_ids, source_candidate_id, source_operation_id,
          confidence, reason, created_at_epoch)
         VALUES ('supersedes', 'trusted', 'memory', 2, 'memory', 1,
                 ?1, ?2, ?3, 0.9, 'cross-project replacement', 20)",
        params![format!("[{event_id}]"), candidate_id, operation_id],
    )?;

    let report = build_truth_report(&conn, &options())?;

    assert_eq!(report.counts.current, 0);
    assert_eq!(report.counts.abstentions, 1);
    assert_eq!(report.counts.supersedes_relations, 1);
    assert_eq!(report.supersedes[0].newer_claim_ref, "memory:2");
    assert_eq!(report.supersedes[0].older_claim_ref, "memory:1");
    assert!(report.lifecycle_mappings.iter().any(|mapping| {
        mapping.object_kind == "trusted_graph_relation"
            && mapping.stored_status == "current"
            && mapping.count == 1
    }));
    Ok(())
}

#[test]
fn subject_accepts_bare_and_composite_user_claim_keys() -> Result<()> {
    let conn = test_conn()?;
    conn.execute(
        "INSERT INTO user_context_claims
         (owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status,
          created_at_epoch, updated_at_epoch)
         VALUES ('repo', '/repo', 'preference', 'editor', 'Use Vim', 1.0,
                 'normal', 'manual', '[]', 'active', 10, 10)",
        [],
    )?;

    for selector in ["editor", "preference:editor"] {
        let mut scoped = options();
        scoped.subject = Some(selector.to_string());
        let report = build_truth_report(&conn, &scoped)?;
        assert_eq!(report.counts.truth_items, 1, "selector {selector}");
        assert_eq!(report.counts.current, 1, "selector {selector}");
    }
    Ok(())
}

#[test]
fn subject_excludes_unrelated_relation_diagnostics() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "stale", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    insert_memory(&conn, 3, "runtime", "stale", 10)?;
    insert_memory(&conn, 4, "runtime", "active", 20)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', 1, 2, 20), ('duplicates', 3, 4, 20)",
        [],
    )?;

    let mut scoped = options();
    scoped.subject = Some("deploy".to_string());
    let report = build_truth_report(&conn, &scoped)?;

    assert_eq!(report.status, "ok");
    assert_eq!(report.counts.supersedes_relations, 1);
    assert_eq!(report.counts.reference_issues, 0);
    assert!(report
        .supersedes
        .iter()
        .all(|link| link.relation_ref == "memory_edge:1"));
    Ok(())
}

#[test]
fn replacement_edges_accept_stale_historical_memory_endpoints() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(&conn, 1, "deploy", "stale", 10)?;
    insert_memory(&conn, 2, "deploy", "active", 20)?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES ('supersedes', 1, 2, 20),
                ('merged_into', 1, 2, 20),
                ('duplicates', 1, 2, 20)",
        [],
    )?;

    let report = build_truth_report(&conn, &options())?;

    assert_eq!(report.status, "ok");
    assert!(report.reference_issues.is_empty());
    Ok(())
}

#[test]
fn user_claim_replacement_checks_the_newer_endpoint() -> Result<()> {
    let conn = test_conn()?;
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status,
          created_at_epoch, updated_at_epoch)
         VALUES (1, 'repo', '/repo', 'preference', 'editor', 'Use Vim', 1.0,
                 'normal', 'manual', '[]', 'superseded', 10, 10)",
        [],
    )?;
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status, supersedes_claim_id,
          created_at_epoch, updated_at_epoch)
         VALUES (2, 'repo', '/repo', 'preference', 'editor', 'Use Helix', 1.0,
                 'normal', 'manual', '[]', 'suppressed', 1, 20, 20)",
        [],
    )?;

    let report = build_truth_report(&conn, &options())?;

    assert_eq!(report.status, "warn");
    assert_eq!(report.reference_issues.len(), 1);
    assert_eq!(report.reference_issues[0].claim_ref, "user_claim:2");
    assert_eq!(
        report.reference_issues[0].stored_status.as_deref(),
        Some("suppressed")
    );
    Ok(())
}

#[test]
fn subject_excludes_unrelated_user_claim_reference_issues() -> Result<()> {
    let conn = test_conn()?;
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status,
          created_at_epoch, updated_at_epoch)
         VALUES (1, 'repo', '/repo', 'preference', 'theme', 'Use dark mode', 1.0,
                 'normal', 'manual', '[]', 'superseded', 10, 10)",
        [],
    )?;
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status, supersedes_claim_id,
          created_at_epoch, updated_at_epoch)
         VALUES (2, 'repo', '/repo', 'preference', 'theme', 'Use light mode', 1.0,
                 'normal', 'manual', '[]', 'suppressed', 1, 20, 20)",
        [],
    )?;

    let mut scoped = options();
    scoped.subject = Some("editor".to_string());
    let report = build_truth_report(&conn, &scoped)?;

    assert_eq!(report.status, "ok");
    assert!(report.supersedes.is_empty());
    assert!(report.reference_issues.is_empty());
    Ok(())
}
