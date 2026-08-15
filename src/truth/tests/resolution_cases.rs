use super::*;
use serde_json::json;

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
