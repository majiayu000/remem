use anyhow::Result;
use rusqlite::Connection;

use super::*;
use crate::user_context::claims::{create_manual_claim, ManualClaimRequest};

fn migrated_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[test]
fn low_risk_explicit_user_statement_can_auto_promote() -> Result<()> {
    let conn = migrated_conn()?;
    let mut req = candidate_request("Prefer concise review notes", true);
    req.claim_key = Some("preference:review-style");

    let result = create_candidate(&conn, &req)?;

    assert_eq!(result.action, "created_claim");
    assert_eq!(result.candidate.review_status, "auto_promoted");
    let claim = result.claim.expect("auto-promote creates claim");
    assert_eq!(claim.status, "active");
    assert_eq!(claim.source_kind, "user_context_candidate");
    assert!(claim.source_refs_json.contains("user_context_candidate"));
    assert_eq!(result.candidate.result_claim_id, Some(claim.id));
    Ok(())
}

#[test]
fn auto_promote_blocks_conflicting_active_claim_key() -> Result<()> {
    let conn = migrated_conn()?;
    let existing = create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "Prefer verbose release notes",
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: Some("preference:release-notes"),
            confidence: 1.0,
            sensitivity: UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;
    let mut req = candidate_request("Prefer concise review notes", true);
    req.claim_key = Some("preference:release-notes");

    let result = create_candidate(&conn, &req)?;

    assert_eq!(result.action, "pending_review");
    assert!(result.claim.is_none());
    assert_eq!(result.candidate.review_status, "pending_review");
    assert_eq!(
        result.candidate.auto_promote_block_reason.as_deref(),
        Some("claim_key_conflict_requires_review")
    );
    assert_eq!(load_claim(&conn, existing.id)?.status, "active");
    Ok(())
}

#[test]
fn auto_promote_rechecks_claim_key_conflict_inside_apply_transaction() -> Result<()> {
    let conn = migrated_conn()?;
    let mut req = candidate_request("Prefer concise review notes", false);
    req.claim_key = Some("preference:release-notes");
    let candidate = create_candidate(&conn, &req)?.candidate;
    let existing = create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "Prefer verbose release notes",
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: Some("preference:release-notes"),
            confidence: 1.0,
            sensitivity: UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;

    let tx = conn.unchecked_transaction()?;
    let result = apply_candidate_tx(&tx, candidate.id, None, "auto_promoted")?;
    tx.commit()?;

    assert_eq!(result.action, "pending_review");
    assert!(result.claim.is_none());
    assert_eq!(result.candidate.review_status, "pending_review");
    assert_eq!(
        result.candidate.auto_promote_block_reason.as_deref(),
        Some("claim_key_conflict_requires_review")
    );
    assert_eq!(load_claim(&conn, existing.id)?.status, "active");
    Ok(())
}

#[test]
fn candidates_require_non_empty_source_refs() -> Result<()> {
    let conn = migrated_conn()?;
    let mut req = candidate_request("Missing source refs", false);
    req.source_refs_json = "[]";
    let err = create_candidate(&conn, &req).expect_err("empty source refs should fail closed");
    assert!(err
        .to_string()
        .contains("candidate source refs must not be empty"));
    Ok(())
}

#[test]
fn blocklisted_candidate_text_or_preview_is_rejected_before_insert() -> Result<()> {
    let conn = migrated_conn()?;
    let secret_text = create_candidate(
        &conn,
        &candidate_request("User's API key is sk-testsecret123456.", false),
    )
    .expect_err("secret-like candidate text should be rejected before insert");
    assert!(secret_text
        .to_string()
        .contains("blocked by non-retention policy"));

    let mut preview_secret = candidate_request("Prefer concise review notes", false);
    preview_secret.source_preview = Some("authorization=Bearer tiny-token");
    let preview_err = create_candidate(&conn, &preview_secret)
        .expect_err("secret-like source preview should be rejected before insert");
    assert!(preview_err
        .to_string()
        .contains("blocked by non-retention policy"));

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[test]
fn sensitive_or_high_risk_candidates_stay_pending_with_block_reason() -> Result<()> {
    let conn = migrated_conn()?;
    let mut sensitive = candidate_request("Sensitive identity detail", true);
    sensitive.sensitivity = UserContextSensitivity::Sensitive;
    let sensitive_result = create_candidate(&conn, &sensitive)?;
    assert_eq!(sensitive_result.candidate.review_status, "pending_review");
    assert_eq!(
        sensitive_result
            .candidate
            .auto_promote_block_reason
            .as_deref(),
        Some("sensitivity_requires_review")
    );
    assert!(sensitive_result.claim.is_none());

    let mut high_risk = candidate_request("Speculative organization claim", true);
    high_risk.risk_class = UserContextCandidateRisk::High;
    let high_risk_result = create_candidate(&conn, &high_risk)?;
    assert_eq!(high_risk_result.candidate.review_status, "pending_review");
    assert_eq!(
        high_risk_result
            .candidate
            .auto_promote_block_reason
            .as_deref(),
        Some("risk_requires_review")
    );
    assert!(high_risk_result.claim.is_none());

    let missing_key = candidate_request("Missing stable key", true);
    let missing_key_result = create_candidate(&conn, &missing_key)?;
    assert_eq!(missing_key_result.candidate.review_status, "pending_review");
    assert_eq!(
        missing_key_result
            .candidate
            .auto_promote_block_reason
            .as_deref(),
        Some("missing_claim_key")
    );
    assert!(missing_key_result.claim.is_none());

    let mut explicit_reason = candidate_request("Needs manual source review", false);
    explicit_reason.auto_promote_block_reason = Some("manual_source_review");
    let explicit_result = create_candidate(&conn, &explicit_reason)?;
    assert_eq!(
        explicit_result
            .candidate
            .auto_promote_block_reason
            .as_deref(),
        Some("manual_source_review")
    );
    Ok(())
}

#[test]
fn third_party_candidates_never_auto_promote() -> Result<()> {
    let conn = migrated_conn()?;
    let mut req = candidate_request("Alice owns release QA for the user's workflow", true);
    req.claim_type = UserContextClaimType::Relationship;
    req.claim_key = Some("relationship:alice-release-qa");
    req.source_kind = "third_party_statement";

    let result = create_candidate(&conn, &req)?;

    assert_eq!(result.action, "pending_review");
    assert!(result.claim.is_none());
    assert_eq!(result.candidate.review_status, "pending_review");
    assert_eq!(
        result.candidate.auto_promote_block_reason.as_deref(),
        Some("third_party_requires_review")
    );
    Ok(())
}

#[test]
fn approve_candidate_requires_stable_claim_key() -> Result<()> {
    let conn = migrated_conn()?;
    let candidate = create_candidate(
        &conn,
        &candidate_request("No stable key should stay review-only", false),
    )?
    .candidate;

    let err = approve_candidate(&conn, candidate.id)
        .expect_err("candidate without claim_key must not become active");
    assert!(err
        .to_string()
        .contains("claim_key is required before applying"));
    assert!(list_claims_for_text(&conn, "No stable key should stay review-only")?.is_empty());
    assert_eq!(
        load_candidate(&conn, candidate.id)?.review_status,
        "pending_review"
    );
    Ok(())
}

#[test]
fn approve_candidate_supersedes_existing_claim_for_same_key() -> Result<()> {
    let conn = migrated_conn()?;
    let existing = create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "Prefer verbose review notes",
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: Some("preference:review-style"),
            confidence: 1.0,
            sensitivity: UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;
    let mut req = candidate_request("Prefer concise review notes", false);
    req.claim_key = Some("preference:review-style");
    let candidate = create_candidate(&conn, &req)?.candidate;

    let approved = approve_candidate(&conn, candidate.id)?;

    assert_eq!(approved.action, "superseded_existing_claim");
    assert_eq!(load_claim(&conn, existing.id)?.status, "superseded");
    let claim = approved.claim.expect("approval creates replacement claim");
    assert_eq!(claim.claim_key, "preference:review-style");
    assert_eq!(claim.supersedes_claim_id, Some(existing.id));
    assert_eq!(approved.candidate.review_status, "approved");
    assert_eq!(approved.candidate.result_claim_id, Some(claim.id));
    Ok(())
}

fn list_claims_for_text(conn: &Connection, text: &str) -> Result<Vec<UserContextClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_key, owner_scope, owner_key, claim_type, claim_key,
                claim_text, confidence, sensitivity, source_kind,
                source_refs_json, status, valid_from_epoch, valid_to_epoch,
                last_confirmed_at_epoch, supersedes_claim_id,
                created_at_epoch, updated_at_epoch
         FROM user_context_claims
         WHERE claim_text = ?1",
    )?;
    let rows = stmt.query_map([text], |row| {
        Ok(UserContextClaim {
            id: row.get(0)?,
            user_key: row.get(1)?,
            owner_scope: row.get(2)?,
            owner_key: row.get(3)?,
            claim_type: row.get(4)?,
            claim_key: row.get(5)?,
            claim_text: row.get(6)?,
            confidence: row.get(7)?,
            sensitivity: row.get(8)?,
            source_kind: row.get(9)?,
            source_refs_json: row.get(10)?,
            status: row.get(11)?,
            valid_from_epoch: row.get(12)?,
            valid_to_epoch: row.get(13)?,
            last_confirmed_at_epoch: row.get(14)?,
            supersedes_claim_id: row.get(15)?,
            created_at_epoch: row.get(16)?,
            updated_at_epoch: row.get(17)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

#[test]
fn approve_candidate_noops_when_matching_active_claim_exists() -> Result<()> {
    let conn = migrated_conn()?;
    let existing = create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "Prefer concise review notes",
            owner_scope: None,
            owner_key: None,
            claim_type: UserContextClaimType::Preference,
            claim_key: Some("preference:review-style"),
            confidence: 1.0,
            sensitivity: UserContextSensitivity::Normal,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;
    let mut req = candidate_request("Prefer concise review notes", false);
    req.claim_key = Some("preference:review-style");
    let candidate = create_candidate(&conn, &req)?.candidate;

    let approved = approve_candidate(&conn, candidate.id)?;

    assert_eq!(approved.action, "noop_existing_claim");
    assert_eq!(approved.candidate.result_claim_id, Some(existing.id));
    assert_eq!(load_claim(&conn, existing.id)?.status, "active");
    Ok(())
}

#[test]
fn edit_reject_and_suppress_candidate_review_paths() -> Result<()> {
    let conn = migrated_conn()?;
    let first = create_candidate(&conn, &candidate_request("Draft review style", false))?.candidate;
    let edited = edit_candidate(
        &conn,
        first.id,
        &CandidateEditRequest {
            text: "Prefer edited review style",
            claim_type: Some(UserContextClaimType::Preference),
            claim_key: Some("preference:edited-review-style"),
            sensitivity: Some(UserContextSensitivity::Normal),
            review_note: Some("approved after edit"),
        },
    )?;
    assert_eq!(edited.candidate.review_status, "edited");
    assert_eq!(
        edited.candidate.review_note.as_deref(),
        Some("approved after edit")
    );
    assert_eq!(
        edited.claim.as_ref().map(|claim| claim.claim_key.as_str()),
        Some("preference:edited-review-style")
    );
    assert_eq!(edited.candidate.claim_text, "Prefer edited review style");
    assert_eq!(
        edited.candidate.claim_key.as_deref(),
        Some("preference:edited-review-style")
    );
    assert_eq!(edited.candidate.claim_type, "preference");
    assert_eq!(edited.candidate.sensitivity, "normal");
    assert_eq!(
        edited.claim.as_ref().map(|claim| claim.claim_text.as_str()),
        Some("Prefer edited review style")
    );
    let stale_reject = reject_candidate(&conn, edited.candidate.id, Some("too late"))
        .expect_err("resolved candidate must not be rejected later");
    assert!(stale_reject
        .to_string()
        .contains("only pending_review or deferred"));
    assert_eq!(
        load_candidate(&conn, edited.candidate.id)?.review_status,
        "edited"
    );

    let rejected =
        create_candidate(&conn, &candidate_request("Reject this context", false))?.candidate;
    let rejected = reject_candidate(&conn, rejected.id, Some("speculative"))?;
    assert_eq!(rejected.review_status, "rejected");
    assert_eq!(rejected.review_note.as_deref(), Some("speculative"));

    let suppressed =
        create_candidate(&conn, &candidate_request("Suppress this context", false))?.candidate;
    let suppressed = suppress_candidate(&conn, suppressed.id, Some("do not suggest"))?;
    assert_eq!(suppressed.review_status, "suppressed");
    assert_eq!(suppressed.review_note.as_deref(), Some("do not suggest"));
    Ok(())
}

fn candidate_request(text: &str, auto_promote: bool) -> CandidateCreateRequest<'_> {
    CandidateCreateRequest {
        text,
        owner_scope: None,
        owner_key: None,
        source_project: Some("/repo"),
        host: Some("codex-cli"),
        session_id: Some("session-1"),
        claim_type: UserContextClaimType::Preference,
        claim_key: None,
        confidence: 0.95,
        sensitivity: UserContextSensitivity::Normal,
        risk_class: UserContextCandidateRisk::Low,
        source_kind: "explicit_user_statement",
        source_refs_json: r#"[{"kind":"captured_event","id":1}]"#,
        source_preview: Some("The user explicitly said they prefer concise review notes."),
        auto_promote,
        auto_promote_block_reason: None,
    }
}
