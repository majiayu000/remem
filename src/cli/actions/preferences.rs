use anyhow::{ensure, Result};
use serde::Serialize;

use crate::{
    db,
    memory::preference,
    user_context::{claims, preference_backfill, recall},
};

use super::shared::resolve_cwd_project;
use crate::cli::query_types::UserClaimsAction;
use crate::cli::types::{PreferenceAction, UserAction};

pub(in crate::cli) fn run_preferences(action: PreferenceAction) -> Result<()> {
    let conn = db::open_db()?;
    let (_, default_project) = resolve_cwd_project();

    match action {
        PreferenceAction::List => preference::list_preferences(&conn, &default_project)?,
        PreferenceAction::Add {
            project,
            global,
            text,
        } => {
            let proj = project.unwrap_or(default_project);
            let id = preference::add_preference(&conn, &proj, &text, global)?;
            let scope_label = if global { "global" } else { "project" };
            println!(
                "Preference added (id={}, scope={}) for project '{}'",
                id, scope_label, proj
            );
        }
        PreferenceAction::Remove { id } => {
            if preference::remove_preference(&conn, id)? {
                println!("Preference {} archived.", id);
            } else {
                println!("Preference {} not found or not a preference type.", id);
            }
        }
    }

    Ok(())
}

pub(in crate::cli) fn run_user(action: UserAction) -> Result<()> {
    match action {
        UserAction::Remember {
            scope,
            owner_key,
            claim_type,
            claim_key,
            sensitivity,
            confidence,
            valid_from_epoch,
            valid_to_epoch,
            json,
            text,
        } => {
            let conn = db::open_db()?;
            let claim = claims::create_manual_claim(
                &conn,
                &claims::ManualClaimRequest {
                    text: &text,
                    owner_scope: Some(scope.db_value()),
                    owner_key: owner_key.as_deref(),
                    claim_type: claim_type.into(),
                    claim_key: claim_key.as_deref(),
                    confidence,
                    sensitivity: sensitivity.into(),
                    valid_from_epoch,
                    valid_to_epoch,
                },
            )?;
            if json {
                print_json(&ClaimOutput {
                    status: "saved",
                    claim,
                })?;
            } else {
                println!("User-context claim saved (id={}).", claim.id);
            }
        }
        UserAction::Backfill { apply, json, limit } => run_user_backfill(apply, json, limit)?,
        UserAction::Claims { action } => {
            let conn = db::open_db()?;
            run_user_claims(&conn, action)?
        }
        UserAction::Summary { action } => super::run_user_summary(action)?,
        UserAction::Profile { action } => super::run_user_profile(action)?,
        UserAction::Review { action } => super::run_user_review(action)?,
        UserAction::Recall {
            query,
            project,
            task_intent,
            current_files,
            host,
            scope,
            owner_key,
            state_keys,
            include_sensitive,
            include_suppressed,
            limit,
            budget_chars,
            json,
        } => run_user_recall(UserRecallCliRequest {
            query,
            project,
            task_intent,
            current_files,
            host,
            owner_scope: scope.db_value().to_string(),
            owner_key,
            state_keys,
            include_sensitive,
            include_suppressed,
            limit,
            budget_chars,
            json,
        })?,
    }
    Ok(())
}

fn run_user_backfill(apply: bool, json: bool, limit: Option<i64>) -> Result<()> {
    if let Some(limit) = limit {
        ensure!(limit > 0, "backfill limit must be positive");
    }

    let request = preference_backfill::UserBackfillRequest { limit };
    let report = if apply {
        let mut conn = db::open_db_no_migrate()?;
        preference_backfill::apply_backfill(&mut conn, &request)?
    } else {
        let conn = db::open_db_read_only()?;
        preference_backfill::preview_backfill(&conn, &request)?
    };
    if json {
        print_json(&report)?;
    } else {
        print_backfill_report(&report);
    }
    Ok(())
}

fn print_backfill_report(report: &preference_backfill::UserBackfillReport) {
    print!("{}", format_backfill_report(report));
}

fn format_backfill_report(report: &preference_backfill::UserBackfillReport) -> String {
    let limit = report
        .limit
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unlimited".to_string());
    let action = if report.applied { "applied" } else { "dry-run" };
    let mut output = String::new();
    output.push_str(&format!(
        "User preference backfill {action}: candidates={}, converted={}, skipped={}, limit={}\n",
        report.candidates.len(),
        report.converted.len(),
        report.skipped.len(),
        limit
    ));
    output.push_str(&format!("{}\n\n", report.message));
    output.push_str("Candidates:\n");
    if report.candidates.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for candidate in &report.candidates {
            output.push_str(&format!("  - memory:{}\n", candidate.memory_id));
        }
    }
    output.push_str("Converted:\n");
    if report.converted.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for converted in &report.converted {
            output.push_str(&format!(
                "  - memory:{} -> claim:{}\n",
                converted.memory_id, converted.claim_id
            ));
        }
    }
    output.push_str("Skipped:\n");
    if report.skipped.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for skipped in &report.skipped {
            output.push_str(&format!(
                "  - memory:{} reason={}\n",
                skipped.memory_id, skipped.reason
            ));
        }
    }
    output
}

struct UserRecallCliRequest {
    query: String,
    project: Option<String>,
    task_intent: Option<String>,
    current_files: Vec<String>,
    host: Option<String>,
    owner_scope: String,
    owner_key: Option<String>,
    state_keys: Vec<String>,
    include_sensitive: bool,
    include_suppressed: bool,
    limit: i64,
    budget_chars: usize,
    json: bool,
}

fn run_user_recall(req: UserRecallCliRequest) -> Result<()> {
    let conn = db::open_db()?;
    let project = req.project.unwrap_or_else(|| resolve_cwd_project().1);
    let result = recall::recall_user_context(
        &conn,
        &recall::UserRecallRequest {
            query: req.query,
            project,
            task_intent: req.task_intent,
            current_files: req.current_files,
            host: req.host,
            owner_scope: Some(req.owner_scope),
            owner_key: req.owner_key,
            state_keys: req.state_keys,
            include_sensitive: req.include_sensitive,
            include_suppressed: req.include_suppressed,
            limit: Some(req.limit),
            budget_chars: Some(req.budget_chars),
        },
    )?;
    if req.json {
        print_json(&result)?;
    } else if result.empty {
        println!("No relevant user context found.");
    } else {
        if let Some(policy) = result.usage_policy {
            println!("{policy}");
            println!();
        }
        println!("{}", result.context);
        if !result.dropped.is_empty() {
            println!();
            println!("Dropped {} candidate(s).", result.dropped.len());
        }
    }
    Ok(())
}

fn run_user_claims(conn: &rusqlite::Connection, action: UserClaimsAction) -> Result<()> {
    match action {
        UserClaimsAction::List {
            scope,
            owner_key,
            include_inactive,
            limit,
            json,
        } => {
            let claims = claims::list_claims(
                conn,
                &claims::ClaimListRequest {
                    owner_scope: scope.map(|scope| scope.db_value()),
                    owner_key: owner_key.as_deref(),
                    include_inactive,
                    limit,
                },
            )?;
            if json {
                print_json(&ClaimListOutput {
                    count: claims.len(),
                    claims,
                })?;
            } else if claims.is_empty() {
                println!("No user-context claims found.");
            } else {
                for claim in claims {
                    println!(
                        "{} [{}:{}] {} ({}, {})",
                        claim.id,
                        claim.claim_type,
                        claim.status,
                        claim.claim_text,
                        claim.owner_scope,
                        claim.owner_key
                    );
                }
            }
        }
        UserClaimsAction::Show { id, json } | UserClaimsAction::Why { id, json } => {
            let claim = claims::load_claim(conn, id)?;
            if json {
                print_json(&ClaimShowOutput { found: true, claim })?;
            } else {
                print_claim_details(&claim);
            }
        }
        UserClaimsAction::Edit {
            id,
            text,
            claim_type,
            claim_key,
            sensitivity,
            valid_from_epoch,
            valid_to_epoch,
            json,
        } => {
            let result = claims::edit_claim(
                conn,
                id,
                &claims::ClaimEditRequest {
                    text: &text,
                    claim_type: claim_type.map(Into::into),
                    claim_key: claim_key.as_deref(),
                    sensitivity: sensitivity.map(Into::into),
                    valid_from_epoch,
                    valid_to_epoch,
                },
            )?;
            if json {
                print_json(&ClaimEditOutput {
                    status: "edited",
                    previous_id: result.previous.id,
                    claim: result.current,
                })?;
            } else {
                println!(
                    "User-context claim {} superseded by {}.",
                    result.previous.id, result.current.id
                );
            }
        }
        UserClaimsAction::Suppress { id, json } => {
            let claim = claims::suppress_claim(conn, id)?;
            print_status("suppressed", claim, json)?;
        }
        UserClaimsAction::Unsuppress { id, json } => {
            let claim = claims::unsuppress_claim(conn, id)?;
            print_status("active", claim, json)?;
        }
        UserClaimsAction::Delete { id, json } => {
            let claim = claims::delete_claim(conn, id)?;
            print_status("deleted", claim, json)?;
        }
    }
    Ok(())
}

fn print_claim_details(claim: &claims::UserContextClaim) {
    println!("ID:           {}", claim.id);
    println!("Status:       {}", claim.status);
    println!("Type:         {}", claim.claim_type);
    println!("Key:          {}", claim.claim_key);
    println!("Sensitivity:  {}", claim.sensitivity);
    println!("Confidence:   {:.3}", claim.confidence);
    println!("Owner:        {}:{}", claim.owner_scope, claim.owner_key);
    println!("User key:     {}", claim.user_key);
    println!("Source kind:  {}", claim.source_kind);
    println!("Source refs:  {}", claim.source_refs_json);
    println!("Created:      {}", format_epoch(claim.created_at_epoch));
    println!("Updated:      {}", format_epoch(claim.updated_at_epoch));
    if let Some(epoch) = claim.valid_from_epoch {
        println!("Valid from:   {}", format_epoch(epoch));
    }
    if let Some(epoch) = claim.valid_to_epoch {
        println!("Valid to:     {}", format_epoch(epoch));
    }
    if let Some(epoch) = claim.last_confirmed_at_epoch {
        println!("Confirmed:    {}", format_epoch(epoch));
    }
    if let Some(id) = claim.supersedes_claim_id {
        println!("Supersedes:   {}", id);
    }
    println!();
    println!("{}", claim.claim_text);
}

fn format_epoch(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

fn print_status(status: &'static str, claim: claims::UserContextClaim, json: bool) -> Result<()> {
    if json {
        print_json(&ClaimOutput { status, claim })?;
    } else {
        println!("User-context claim {} is now {}.", claim.id, status);
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Serialize)]
struct ClaimOutput {
    status: &'static str,
    claim: claims::UserContextClaim,
}

#[derive(Serialize)]
struct ClaimListOutput {
    count: usize,
    claims: Vec<claims::UserContextClaim>,
}

#[derive(Serialize)]
struct ClaimShowOutput {
    found: bool,
    claim: claims::UserContextClaim,
}

#[derive(Serialize)]
struct ClaimEditOutput {
    status: &'static str,
    previous_id: i64,
    claim: claims::UserContextClaim,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::query_types::{
        UserClaimScopeArg, UserClaimSensitivityArg, UserClaimTypeArg, UserClaimsAction,
    };

    #[test]
    fn user_actions_write_and_govern_claims() -> Result<()> {
        let _dir = crate::db::test_support::ScopedTestDataDir::new("user-context-cli-actions");
        run_user(UserAction::Remember {
            scope: UserClaimScopeArg::User,
            owner_key: None,
            claim_type: UserClaimTypeArg::Preference,
            claim_key: Some("pref:reviews".to_string()),
            sensitivity: UserClaimSensitivityArg::Normal,
            confidence: 0.9,
            valid_from_epoch: None,
            valid_to_epoch: None,
            json: false,
            text: "Prefer product-first review".to_string(),
        })?;

        let conn = db::open_db()?;
        let claim = claims::load_claim(&conn, 1)?;
        assert_eq!(claim.status, "active");
        assert_eq!(claim.owner_key, claims::DEFAULT_OWNER_KEY);

        run_user(UserAction::Claims {
            action: UserClaimsAction::List {
                scope: None,
                owner_key: None,
                include_inactive: false,
                limit: 50,
                json: true,
            },
        })?;
        run_user(UserAction::Claims {
            action: UserClaimsAction::Show { id: 1, json: true },
        })?;
        run_user(UserAction::Claims {
            action: UserClaimsAction::Why { id: 1, json: true },
        })?;

        run_user(UserAction::Claims {
            action: UserClaimsAction::Edit {
                id: 1,
                text: "Prefer architecture-first review".to_string(),
                claim_type: Some(UserClaimTypeArg::Preference),
                claim_key: Some("pref:reviews".to_string()),
                sensitivity: Some(UserClaimSensitivityArg::Normal),
                valid_from_epoch: None,
                valid_to_epoch: None,
                json: false,
            },
        })?;
        assert_eq!(claims::load_claim(&conn, 1)?.status, "superseded");
        assert_eq!(claims::load_claim(&conn, 2)?.supersedes_claim_id, Some(1));

        run_user(UserAction::Claims {
            action: UserClaimsAction::Suppress { id: 2, json: false },
        })?;
        assert_eq!(claims::load_claim(&conn, 2)?.status, "suppressed");
        run_user(UserAction::Claims {
            action: UserClaimsAction::Unsuppress { id: 2, json: false },
        })?;
        assert_eq!(claims::load_claim(&conn, 2)?.status, "active");
        run_user(UserAction::Claims {
            action: UserClaimsAction::Delete { id: 2, json: false },
        })?;
        assert_eq!(claims::load_claim(&conn, 2)?.status, "deleted");
        Ok(())
    }

    #[test]
    fn user_recall_action_returns_json_shape() -> Result<()> {
        let _dir = crate::db::test_support::ScopedTestDataDir::new("user-recall-cli-action");
        let conn = db::open_db()?;
        claims::create_manual_claim(
            &conn,
            &claims::ManualClaimRequest {
                text: "Prefer recall CLI JSON",
                owner_scope: None,
                owner_key: None,
                claim_type: claims::UserContextClaimType::Preference,
                claim_key: None,
                confidence: 1.0,
                sensitivity: claims::UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        drop(conn);

        run_user(UserAction::Recall {
            query: "recall CLI".to_string(),
            project: Some("/repo".to_string()),
            task_intent: None,
            current_files: Vec::new(),
            host: None,
            scope: crate::cli::query_types::UserClaimScopeArg::User,
            owner_key: None,
            state_keys: Vec::new(),
            include_sensitive: false,
            include_suppressed: false,
            limit: 5,
            budget_chars: 1_000,
            json: true,
        })
    }

    #[test]
    fn user_backfill_dry_run_requires_existing_db_without_creating_it() {
        let dir = crate::db::test_support::ScopedTestDataDir::new("user-backfill-cli-dry-run");
        assert!(!dir.db_path().exists());

        let err = run_user(UserAction::Backfill {
            apply: false,
            json: true,
            limit: Some(5),
        })
        .expect_err("dry-run should require an existing database");

        assert!(err.to_string().contains("database not found"));
        assert!(!dir.db_path().exists());
    }

    #[test]
    fn user_backfill_apply_requires_existing_db_without_creating_it() {
        let dir = crate::db::test_support::ScopedTestDataDir::new("user-backfill-cli-apply");
        assert!(!dir.db_path().exists());
        let err = run_user(UserAction::Backfill {
            apply: true,
            json: false,
            limit: None,
        })
        .expect_err("apply should require an existing database");
        assert!(err.to_string().contains("database not found"));
        assert!(!dir.db_path().exists());
    }

    #[test]
    fn user_backfill_rejects_non_positive_limit() {
        let err = run_user(UserAction::Backfill {
            apply: false,
            json: false,
            limit: Some(0),
        })
        .expect_err("zero limit should be rejected");
        assert!(err.to_string().contains("limit must be positive"));
    }

    #[test]
    fn user_backfill_human_report_includes_row_level_audit() {
        let dry_run = preference_backfill::UserBackfillReport {
            applied: false,
            limit: Some(10),
            candidates: vec![preference_backfill::UserBackfillCandidate { memory_id: 101 }],
            converted: Vec::new(),
            skipped: vec![preference_backfill::UserBackfillSkipped {
                memory_id: 202,
                reason: "duplicate".to_string(),
            }],
            message: "Dry-run only; rerun with --apply to convert candidates.".to_string(),
        };

        let dry_run_output = format_backfill_report(&dry_run);
        assert!(dry_run_output.contains("candidates=1"));
        assert!(dry_run_output.contains("limit=10"));
        assert!(dry_run_output.contains("Candidates:\n  - memory:101\n"));
        assert!(dry_run_output.contains("Converted:\n  (none)\n"));
        assert!(dry_run_output.contains("Skipped:\n  - memory:202 reason=duplicate\n"));

        let applied = preference_backfill::UserBackfillReport {
            applied: true,
            limit: None,
            candidates: Vec::new(),
            converted: vec![preference_backfill::UserBackfillConverted {
                memory_id: 101,
                claim_id: 303,
            }],
            skipped: Vec::new(),
            message: "User preference backfill applied.".to_string(),
        };

        let applied_output = format_backfill_report(&applied);
        assert!(applied_output.contains("limit=unlimited"));
        assert!(applied_output.contains("Candidates:\n  (none)\n"));
        assert!(applied_output.contains("Converted:\n  - memory:101 -> claim:303\n"));
        assert!(applied_output.contains("Skipped:\n  (none)\n"));
    }

    #[test]
    fn user_json_outputs_match_subcommand_shapes() -> Result<()> {
        let _dir = crate::db::test_support::ScopedTestDataDir::new("user-context-json-shapes");
        let conn = db::open_db()?;
        let claim = claims::create_manual_claim(
            &conn,
            &claims::ManualClaimRequest {
                text: "Prefer exact JSON contracts",
                owner_scope: None,
                owner_key: None,
                claim_type: claims::UserContextClaimType::Preference,
                claim_key: Some("pref:json"),
                confidence: 1.0,
                sensitivity: claims::UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;

        let status = serde_json::to_value(&ClaimOutput {
            status: "saved",
            claim: claim.clone(),
        })?;
        assert!(status.get("status").is_some());
        assert!(status.get("claim").is_some());

        let list = serde_json::to_value(&ClaimListOutput {
            count: 1,
            claims: vec![claim.clone()],
        })?;
        assert!(list.get("count").is_some());
        assert!(list.get("claims").is_some());
        assert!(list.get("status").is_none());

        let show = serde_json::to_value(&ClaimShowOutput {
            found: true,
            claim: claim.clone(),
        })?;
        assert!(show.get("found").is_some());
        assert!(show.get("claim").is_some());
        assert!(show.get("status").is_none());

        let edit = serde_json::to_value(&ClaimEditOutput {
            status: "edited",
            previous_id: claim.id,
            claim,
        })?;
        assert!(edit.get("status").is_some());
        assert!(edit.get("previous_id").is_some());
        assert!(edit.get("claim").is_some());
        Ok(())
    }
}
