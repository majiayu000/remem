use anyhow::Result;
use serde::Serialize;

use crate::{
    cli::query_types::UserReviewAction,
    db,
    user_context::{candidates, claims},
};

pub(in crate::cli) fn run_user_review(action: UserReviewAction) -> Result<()> {
    let conn = db::open_db()?;
    match action {
        UserReviewAction::Inbox {
            include_resolved,
            status,
            limit,
            json,
        } => {
            let candidates = candidates::list_candidates(
                &conn,
                &candidates::CandidateListRequest {
                    review_status: status.as_deref(),
                    include_resolved,
                    limit,
                },
            )?;
            if json {
                print_json(&CandidateListOutput {
                    count: candidates.len(),
                    candidates,
                })?;
            } else if candidates.is_empty() {
                println!("No user-context candidates found.");
            } else {
                for candidate in candidates {
                    print_candidate_summary(&candidate);
                }
            }
        }
        UserReviewAction::Approve { id, json } => {
            let result = candidates::approve_candidate(&conn, id)?;
            print_apply_result("approved", result, json)?;
        }
        UserReviewAction::Edit {
            id,
            text,
            claim_type,
            claim_key,
            sensitivity,
            note,
            json,
        } => {
            let result = candidates::edit_candidate(
                &conn,
                id,
                &candidates::CandidateEditRequest {
                    text: &text,
                    claim_type: claim_type.map(Into::into),
                    claim_key: claim_key.as_deref(),
                    sensitivity: sensitivity.map(Into::into),
                    review_note: note.as_deref(),
                },
            )?;
            print_apply_result("edited", result, json)?;
        }
        UserReviewAction::Reject { id, note, json } => {
            let candidate = candidates::reject_candidate(&conn, id, note.as_deref())?;
            print_status("rejected", candidate, json)?;
        }
        UserReviewAction::Suppress { id, note, json } => {
            let candidate = candidates::suppress_candidate(&conn, id, note.as_deref())?;
            print_status("suppressed", candidate, json)?;
        }
    }
    Ok(())
}

fn print_candidate_summary(candidate: &candidates::UserContextCandidate) {
    println!(
        "{} [{}:{} risk={} sensitivity={} confidence={:.3}] {}",
        candidate.id,
        candidate.claim_type,
        candidate.review_status,
        candidate.risk_class,
        candidate.sensitivity,
        candidate.confidence,
        candidate.claim_text
    );
    if let Some(reason) = candidate.auto_promote_block_reason.as_deref() {
        println!("  block_reason: {reason}");
    }
    if let Some(preview) = candidate.source_preview.as_deref() {
        println!("  source: {preview}");
    }
}

fn print_apply_result(
    status: &'static str,
    result: candidates::CandidateApplyResult,
    json: bool,
) -> Result<()> {
    if json {
        print_json(&CandidateApplyOutput {
            status,
            action: result.action,
            candidate: result.candidate,
            claim: result.claim,
        })?;
    } else if let Some(claim) = result.claim {
        println!(
            "User-context candidate {} is now {}; claim {} {}.",
            result.candidate.id, status, claim.id, result.action
        );
    } else {
        println!(
            "User-context candidate {} is now {}.",
            result.candidate.id, status
        );
    }
    Ok(())
}

fn print_status(
    status: &'static str,
    candidate: candidates::UserContextCandidate,
    json: bool,
) -> Result<()> {
    if json {
        print_json(&CandidateStatusOutput { status, candidate })?;
    } else {
        println!("User-context candidate {} is now {}.", candidate.id, status);
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Serialize)]
struct CandidateListOutput {
    count: usize,
    candidates: Vec<candidates::UserContextCandidate>,
}

#[derive(Serialize)]
struct CandidateApplyOutput {
    status: &'static str,
    action: String,
    candidate: candidates::UserContextCandidate,
    claim: Option<claims::UserContextClaim>,
}

#[derive(Serialize)]
struct CandidateStatusOutput {
    status: &'static str,
    candidate: candidates::UserContextCandidate,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::query_types::UserReviewAction;
    use crate::user_context::candidates::{CandidateCreateRequest, UserContextCandidateRisk};
    use crate::user_context::claims::{UserContextClaimType, UserContextSensitivity};

    #[test]
    fn user_review_actions_apply_and_govern_candidates() -> Result<()> {
        let _dir = crate::db::test_support::ScopedTestDataDir::new("user-review-cli-actions");
        let conn = db::open_db()?;
        let approved = candidates::create_candidate(&conn, &candidate_request("Approve me"))?
            .candidate
            .id;
        let rejected = candidates::create_candidate(&conn, &candidate_request("Reject me"))?
            .candidate
            .id;
        let suppressed = candidates::create_candidate(&conn, &candidate_request("Suppress me"))?
            .candidate
            .id;
        drop(conn);

        run_user_review(UserReviewAction::Inbox {
            include_resolved: false,
            status: None,
            limit: 10,
            json: true,
        })?;
        run_user_review(UserReviewAction::Approve {
            id: approved,
            json: true,
        })?;
        run_user_review(UserReviewAction::Reject {
            id: rejected,
            note: Some("speculative".to_string()),
            json: true,
        })?;
        run_user_review(UserReviewAction::Suppress {
            id: suppressed,
            note: Some("not useful".to_string()),
            json: true,
        })?;

        let conn = db::open_db()?;
        assert_eq!(
            candidates::load_candidate(&conn, approved)?.review_status,
            "approved"
        );
        assert!(candidates::load_candidate(&conn, approved)?
            .result_claim_id
            .is_some());
        assert_eq!(
            candidates::load_candidate(&conn, rejected)?.review_status,
            "rejected"
        );
        assert_eq!(
            candidates::load_candidate(&conn, suppressed)?.review_status,
            "suppressed"
        );
        Ok(())
    }

    fn candidate_request(text: &str) -> CandidateCreateRequest<'_> {
        CandidateCreateRequest {
            text,
            owner_scope: None,
            owner_key: None,
            source_project: Some("/repo"),
            host: Some("codex-cli"),
            session_id: Some("session-1"),
            claim_type: UserContextClaimType::Preference,
            claim_key: Some("preference:cli-review-test"),
            confidence: 0.8,
            sensitivity: UserContextSensitivity::Normal,
            risk_class: UserContextCandidateRisk::Low,
            source_kind: "session_summary",
            source_refs_json: r#"[{"kind":"session_summary","id":1}]"#,
            source_preview: Some("candidate preview"),
            auto_promote: false,
            auto_promote_block_reason: None,
        }
    }
}
