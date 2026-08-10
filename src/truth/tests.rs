//! Golden fixtures for the CurrentTruth projection (GH933 Phase A).
//!
//! Every fixture builds a real migrated in-memory database and asserts the
//! deterministic projection output, including selection reasons.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::json;

use super::{
    project_current_truth, project_user_claim_truth, EvidenceTrust, TruthQuery,
    TruthSelectionReason, ValidityState,
};

fn test_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[allow(clippy::too_many_arguments)]
fn insert_memory(
    conn: &Connection,
    id: i64,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    status: &str,
    branch: Option<&str>,
    created_at: i64,
    updated_at: i64,
    evidence_event_ids: Option<&str>,
) -> Result<()> {
    let proof_event_id = 8_000_000 + id;
    let candidate_id = 7_000_000 + id;
    let state_key_id = 6_000_000 + id;
    insert_captured_event(conn, proof_event_id, Some("assistant"), None, created_at)?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 9001, 'project', 'decision', ?2, ?3, ?4,
                 0.9, 'low', 'accepted', ?5, ?5)",
        params![
            candidate_id,
            topic_key,
            content,
            format!("[{proof_event_id}]"),
            created_at
        ],
    )?;
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', ?2, 'decision', ?3, ?4, ?4)",
        params![
            state_key_id,
            project,
            format!("truth-fixture-{id}"),
            created_at
        ],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, branch, evidence_event_ids,
          source_candidate_id, state_key_id)
         VALUES (?1, ?2, ?3, ?4, ?5, 'decision', ?6, ?7, ?8, ?9, ?10,
                 ?11, ?12)",
        params![
            id,
            project,
            topic_key,
            title,
            content,
            created_at,
            updated_at,
            status,
            branch,
            evidence_event_ids,
            candidate_id,
            state_key_id,
        ],
    )?;
    Ok(())
}

fn insert_captured_event(
    conn: &Connection,
    id: i64,
    role: Option<&str>,
    tool_name: Option<&str>,
    created_at: i64,
) -> Result<()> {
    seed_capture_parents(conn)?;
    conn.execute(
        "INSERT INTO captured_events
         (id, host_id, workspace_id, project_id, session_row_id, session_id, event_id,
          event_type, role, tool_name, content_hash, retention_class,
          created_at_epoch, inserted_at_epoch)
         VALUES (?1, 9001, 9001, 9001, 9001, 'truth-session', ?2,
                 'message', ?3, ?4, ?5, 'normal', ?6, ?6)",
        params![
            id,
            format!("event-{id}"),
            role,
            tool_name,
            format!("hash-{id}"),
            created_at
        ],
    )?;
    Ok(())
}

fn seed_capture_parents(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO hosts (id, name, created_at_epoch)
         VALUES (9001, 'truth-host', 0);
         INSERT OR IGNORE INTO workspaces (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (9001, '/truth-ws', 0, 0);
         INSERT OR IGNORE INTO projects
         (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (9001, 9001, '/truth-ws', 'truth', 0, 0);
         INSERT OR IGNORE INTO sessions
         (id, host_id, workspace_id, project_id, session_id, last_seen_at_epoch, status)
         VALUES (9001, 9001, 9001, 9001, 'truth-session', 0, 'active');",
    )?;
    Ok(())
}

#[test]
fn legacy_unverified_memory_is_rejected_by_current_truth() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        991,
        "/repo",
        Some("g2-legacy"),
        "legacy",
        "legacy payload",
        "active",
        None,
        10,
        10,
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET source_trust_class = 'local_tool_output', source_candidate_id = NULL,
             confidence = NULL, valid_from_epoch = NULL, state_key_id = NULL
         WHERE id = 991",
        [],
    )?;
    let projection = project_current_truth(&conn, &query("/repo"))?;
    let truth = projection
        .truths
        .iter()
        .find(|truth| truth.subject_key == "g2-legacy")
        .unwrap();
    assert!(truth.claim.is_none());
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::InsufficientEvidence
    );
    assert_eq!(truth.rejected, vec!["memory:991"]);
    Ok(())
}

#[test]
fn current_truth_uses_one_reference_epoch_for_visibility_and_resolution() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("epoch-boundary"),
        "current",
        "current claim",
        "active",
        None,
        10,
        10,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("epoch-boundary"),
        "future",
        "future claim",
        "active",
        None,
        30,
        30,
        None,
    )?;

    let query = query("proj");
    let projection =
        super::projection::project_current_truth_at_reference_epoch(&conn, &query, 20)?;
    let truth = projection
        .truths
        .iter()
        .find(|truth| truth.subject_key == "epoch-boundary")
        .expect("epoch-boundary truth");
    assert_eq!(projection.as_of_epoch, None);
    assert_eq!(
        truth
            .claim
            .as_ref()
            .map(|claim| claim.canonical_ref.as_str()),
        Some("memory:1")
    );
    assert_eq!(truth.rejected, vec!["memory:2"]);
    Ok(())
}

fn insert_edge(
    conn: &Connection,
    edge_type: &str,
    from_memory_id: i64,
    to_memory_id: i64,
    created_at: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_edges (edge_type, from_memory_id, to_memory_id, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4)",
        params![edge_type, from_memory_id, to_memory_id, created_at],
    )?;
    Ok(())
}

fn query(project: &str) -> TruthQuery {
    TruthQuery {
        project: project.to_string(),
        branch: None,
        as_of_epoch: None,
        subject_key: None,
    }
}

#[test]
fn explicit_supersedes_beats_recency() -> Result<()> {
    let conn = test_conn()?;
    // The superseded row has the NEWER updated_at to prove the explicit
    // relation, not recency, decides.
    insert_memory(
        &conn,
        1,
        "proj",
        Some("deploy-target"),
        "Deploy target",
        "staging",
        "active",
        None,
        100,
        400,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("deploy-target"),
        "Deploy target",
        "production",
        "active",
        None,
        180,
        180,
        None,
    )?;
    // Stored replacement direction: from=old, to=new.
    insert_edge(&conn, "supersedes", 1, 2, 200)?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    assert_eq!(projection.truths.len(), 1);
    let truth = &projection.truths[0];
    assert_eq!(truth.validity, ValidityState::Current);
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::ExplicitSupersedes
    );
    let claim = truth.claim.as_ref().expect("winner claim");
    assert_eq!(claim.canonical_ref, "memory:2");
    assert!(truth.rejected.contains(&"memory:1".to_string()));
    assert_eq!(truth.supporting_relations.len(), 1);
    assert_eq!(truth.supporting_relations[0].from_ref, "memory:2");
    assert_eq!(truth.supporting_relations[0].to_ref, "memory:1");
    Ok(())
}

#[test]
fn as_of_returns_the_then_current_now_superseded_decision() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("deploy-target"),
        "Deploy target",
        "staging",
        "active",
        None,
        100,
        100,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("deploy-target"),
        "Deploy target",
        "production",
        "active",
        None,
        180,
        180,
        None,
    )?;
    insert_edge(&conn, "supersedes", 1, 2, 200)?;

    // Before the replacement existed, the old decision was the truth.
    let mut historical = query("proj");
    historical.as_of_epoch = Some(150);
    let projection = project_current_truth(&conn, &historical)?;
    let truth = &projection.truths[0];
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::OnlySurvivingClaim
    );
    assert_eq!(
        truth
            .claim
            .as_ref()
            .expect("historical claim")
            .canonical_ref,
        "memory:1"
    );

    // Today the supersedes edge applies.
    let projection = project_current_truth(&conn, &query("proj"))?;
    assert_eq!(
        projection.truths[0]
            .claim
            .as_ref()
            .expect("current claim")
            .canonical_ref,
        "memory:2"
    );
    Ok(())
}

#[test]
fn two_sources_supporting_one_claim_keep_both_evidence_refs() -> Result<()> {
    let conn = test_conn()?;
    insert_captured_event(&conn, 501, Some("user"), None, 90)?;
    insert_captured_event(&conn, 502, None, Some("Bash"), 95)?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("db-choice"),
        "Database",
        "sqlite",
        "active",
        None,
        100,
        100,
        Some("[501,502]"),
    )?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    let truth = &projection.truths[0];
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::OnlySurvivingClaim
    );
    let refs: Vec<&str> = truth
        .evidence
        .iter()
        .map(|evidence| evidence.evidence_ref.as_str())
        .collect();
    assert_eq!(refs, ["captured_event:501", "captured_event:502"]);
    assert!(truth
        .evidence
        .iter()
        .all(|evidence| evidence.trust == EvidenceTrust::Verified));
    Ok(())
}

#[test]
fn unresolved_conflict_returns_contradicted_not_a_silent_pick() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("api-style"),
        "API style",
        "REST",
        "active",
        None,
        100,
        100,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("api-style"),
        "API style",
        "gRPC",
        "active",
        None,
        150,
        150,
        None,
    )?;
    insert_edge(&conn, "conflicts", 1, 2, 160)?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    let truth = &projection.truths[0];
    assert_eq!(truth.validity, ValidityState::Contradicted);
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::UnresolvedConflict
    );
    assert!(truth.claim.is_none(), "conflict must not fold to one side");
    assert_eq!(truth.conflicting_claims.len(), 2);
    assert_eq!(truth.contradicting_relations.len(), 1);
    Ok(())
}

#[test]
fn verified_evidence_beats_newer_model_generated_claim() -> Result<()> {
    let conn = test_conn()?;
    insert_captured_event(&conn, 601, Some("user"), None, 90)?;
    // Verified-backed claim is OLDER; model-generated claim is newer.
    insert_memory(
        &conn,
        1,
        "proj",
        Some("build-tool"),
        "Build tool",
        "cargo",
        "active",
        None,
        100,
        100,
        Some("[601]"),
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("build-tool"),
        "Build tool",
        "bazel",
        "active",
        None,
        300,
        300,
        None,
    )?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    let truth = &projection.truths[0];
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::VerifiedEvidencePreferred
    );
    assert_eq!(
        truth.claim.as_ref().expect("winner").canonical_ref,
        "memory:1"
    );
    assert!(truth.rejected.contains(&"memory:2".to_string()));
    Ok(())
}

#[test]
fn branches_keep_isolated_current_truth() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("feature-flag"),
        "Flag",
        "on",
        "active",
        Some("branch-a"),
        100,
        100,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("feature-flag"),
        "Flag",
        "off",
        "active",
        Some("branch-b"),
        100,
        100,
        None,
    )?;

    let mut on_a = query("proj");
    on_a.branch = Some("branch-a".to_string());
    let projection = project_current_truth(&conn, &on_a)?;
    assert_eq!(projection.truths.len(), 1);
    assert_eq!(
        projection.truths[0]
            .claim
            .as_ref()
            .expect("branch a")
            .canonical_ref,
        "memory:1"
    );

    let mut on_b = query("proj");
    on_b.branch = Some("branch-b".to_string());
    let projection = project_current_truth(&conn, &on_b)?;
    assert_eq!(
        projection.truths[0]
            .claim
            .as_ref()
            .expect("branch b")
            .canonical_ref,
        "memory:2"
    );
    Ok(())
}

#[test]
fn scope_mismatch_never_leaks_other_projects() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj-b",
        Some("secret-topic"),
        "Secret",
        "b-only",
        "active",
        None,
        100,
        100,
        None,
    )?;

    let projection = project_current_truth(&conn, &query("proj-a"))?;
    assert!(projection.truths.is_empty(), "no cross-project leakage");
    Ok(())
}

#[test]
fn stale_only_group_abstains_instead_of_answering() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("old-path"),
        "Path",
        "src/old.rs",
        "stale",
        None,
        100,
        100,
        None,
    )?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    let truth = &projection.truths[0];
    assert_eq!(truth.validity, ValidityState::Unknown);
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::InsufficientEvidence
    );
    assert!(truth.claim.is_none());
    assert_eq!(truth.rejected, vec!["memory:1".to_string()]);
    Ok(())
}

#[test]
fn archived_and_deleted_rows_never_become_current_truth() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("t"),
        "T",
        "archived fact",
        "archived",
        None,
        100,
        100,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("t"),
        "T",
        "deleted fact",
        "deleted",
        None,
        110,
        110,
        None,
    )?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    let truth = &projection.truths[0];
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::InsufficientEvidence
    );
    assert!(truth.claim.is_none());
    Ok(())
}

#[test]
fn same_tier_same_timestamp_is_contradicted() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("tie"),
        "Tie",
        "left",
        "active",
        None,
        100,
        100,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("tie"),
        "Tie",
        "right",
        "active",
        None,
        100,
        100,
        None,
    )?;

    let projection = project_current_truth(&conn, &query("proj"))?;
    let truth = &projection.truths[0];
    assert_eq!(truth.validity, ValidityState::Contradicted);
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::UnresolvedConflict
    );
    assert_eq!(truth.conflicting_claims.len(), 2);
    Ok(())
}

#[test]
fn user_claim_supersedes_link_resolves_deterministically() -> Result<()> {
    let conn = test_conn()?;
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status,
          created_at_epoch, updated_at_epoch)
         VALUES (1, 'user', 'user:default', 'preference', 'editor', 'uses vim', 0.9,
                 'normal', 'manual', '[]', 'active', 100, 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status, supersedes_claim_id,
          created_at_epoch, updated_at_epoch)
         VALUES (2, 'user', 'user:default', 'preference', 'editor', 'uses helix', 0.9,
                 'normal', 'manual', '[]', 'active', 1, 200, 200)",
        [],
    )?;
    // Suppressed claims stay policy-hidden even when newest.
    conn.execute(
        "INSERT INTO user_context_claims
         (id, owner_scope, owner_key, claim_type, claim_key, claim_text, confidence,
          sensitivity, source_kind, source_refs_json, status,
          created_at_epoch, updated_at_epoch)
         VALUES (3, 'user', 'user:default', 'preference', 'editor', 'uses emacs', 0.9,
                 'normal', 'manual', '[]', 'suppressed', 300, 300)",
        [],
    )?;

    let projection = project_user_claim_truth(&conn, "user", "user:default", None)?;
    assert_eq!(projection.truths.len(), 1);
    let truth = &projection.truths[0];
    assert_eq!(
        truth.selected_reason,
        TruthSelectionReason::ExplicitSupersedes
    );
    let claim = truth.claim.as_ref().expect("winner");
    assert_eq!(claim.canonical_ref, "user_claim:2");
    assert!(truth.rejected.contains(&"user_claim:1".to_string()));
    assert!(truth.rejected.contains(&"user_claim:3".to_string()));
    Ok(())
}

#[test]
fn golden_projection_shape_is_versioned_and_stable() -> Result<()> {
    let conn = test_conn()?;
    insert_memory(
        &conn,
        1,
        "proj",
        Some("deploy-target"),
        "Deploy target",
        "staging",
        "active",
        None,
        100,
        100,
        None,
    )?;
    insert_memory(
        &conn,
        2,
        "proj",
        Some("deploy-target"),
        "Deploy target",
        "production",
        "active",
        None,
        180,
        180,
        None,
    )?;
    insert_edge(&conn, "supersedes", 1, 2, 200)?;

    let mut fixed = query("proj");
    fixed.as_of_epoch = Some(300);
    let projection = project_current_truth(&conn, &fixed)?;
    let actual = serde_json::to_value(&projection)?;
    let expected = json!({
        "projection_version": 1,
        "project": "proj",
        "branch": null,
        "as_of_epoch": 300,
        "truths": [{
            "subject_key": "deploy-target",
            "claim": {
                "canonical_ref": "memory:2",
                "source": "memory",
                "subject_key": "deploy-target",
                "statement": "Deploy target: production",
                "scope": "proj",
                "branch": null,
                "lifecycle": {
                    "publication": "active",
                    "validity": "current",
                    "retention": "live",
                    "visibility": "visible"
                },
                "valid_from_epoch": null,
                "valid_to_epoch": null,
                "created_at_epoch": 180,
                "updated_at_epoch": 180,
                "evidence": []
            },
            "validity": "current",
            "evidence": [],
            "supporting_relations": [{
                "relation_ref": "memory_edge:1",
                "kind": "supersedes",
                "from_ref": "memory:2",
                "to_ref": "memory:1",
                "created_at_epoch": 200,
                "valid_from_epoch": null,
                "valid_to_epoch": null
            }],
            "contradicting_relations": [],
            "rejected": ["memory:1"],
            "conflicting_claims": [],
            "selected_reason": "explicit_supersedes"
        }]
    });
    assert_eq!(actual, expected);
    Ok(())
}
