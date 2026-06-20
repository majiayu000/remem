use anyhow::Result;
use serde::Serialize;

use crate::{
    db,
    user_context::summary::{self, SummaryEditRequest, SummaryRequest, UserContextSummary},
};

use super::shared::resolve_cwd_project;
use crate::cli::query_types::UserSummaryAction;

pub(in crate::cli) fn run_user_summary(action: UserSummaryAction) -> Result<()> {
    let conn = db::open_db()?;
    match action {
        UserSummaryAction::Show {
            project,
            scope,
            owner_key,
            json,
        } => {
            let project = resolve_project(project);
            let req = request(scope.db_value(), owner_key.as_deref(), &project);
            let summary = summary::load_active_summary(&conn, &req)?;
            if json {
                print_json(&SummaryShowOutput {
                    found: summary.is_some(),
                    summary,
                })?;
            } else if let Some(summary) = summary {
                print_summary(&summary);
            } else {
                println!("No user-context summary found.");
            }
        }
        UserSummaryAction::Refresh {
            project,
            scope,
            owner_key,
            json,
        } => {
            let project = resolve_project(project);
            let req = request(scope.db_value(), owner_key.as_deref(), &project);
            let summary = summary::refresh_summary(&conn, &req)?;
            print_status("refreshed", summary, json)?;
        }
        UserSummaryAction::Edit {
            project,
            scope,
            owner_key,
            text,
            json,
        } => {
            let project = resolve_project(project);
            let summary = summary::edit_summary(
                &conn,
                &SummaryEditRequest {
                    owner_scope: Some(scope.db_value()),
                    owner_key: owner_key.as_deref(),
                    project: &project,
                    text: &text,
                },
            )?;
            print_status("edited", summary, json)?;
        }
        UserSummaryAction::Sources {
            project,
            scope,
            owner_key,
            include_excluded,
            json,
        } => {
            let project = resolve_project(project);
            let req = request(scope.db_value(), owner_key.as_deref(), &project);
            let sources = summary::load_summary_sources(&conn, &req, include_excluded)?;
            if json {
                print_json(&sources)?;
            } else {
                print_sources(&sources);
            }
        }
    }
    Ok(())
}

fn resolve_project(project: Option<String>) -> String {
    project.unwrap_or_else(|| resolve_cwd_project().1)
}

fn request<'a>(
    owner_scope: &'a str,
    owner_key: Option<&'a str>,
    project: &'a str,
) -> SummaryRequest<'a> {
    SummaryRequest {
        owner_scope: Some(owner_scope),
        owner_key,
        project,
    }
}

fn print_summary(summary: &UserContextSummary) {
    println!(
        "User-context summary {} (version {}, {}, {}:{})",
        summary.id, summary.version, summary.status, summary.owner_scope, summary.owner_key
    );
    println!(
        "Sources: {} claim(s), {} memory/memories, {} activity ref(s)",
        summary.source_claim_ids.len(),
        summary.source_memory_ids.len(),
        summary.source_activity_refs.len()
    );
    println!();
    println!("{}", summary.summary_text);
}

fn print_sources(sources: &summary::SummarySources) {
    match &sources.summary {
        Some(summary) => println!("Summary: {} (version {})", summary.id, summary.version),
        None => println!("Summary: none"),
    }
    println!("Claims:");
    for claim in &sources.included_claims {
        println!("  {} [{}] {}", claim.id, claim.claim_type, claim.claim_text);
    }
    println!("Memories:");
    for memory in &sources.included_memories {
        println!("  {} [{}] {}", memory.id, memory.memory_type, memory.title);
    }
    println!("Activity:");
    for activity in &sources.included_activity_refs {
        println!("  {}:{} {}", activity.kind, activity.id, activity.label);
    }
    if !sources.dropped_claims.is_empty() {
        println!("Excluded claims:");
        for dropped in &sources.dropped_claims {
            println!("  {}:{} {}", dropped.kind, dropped.id, dropped.reason);
        }
    }
}

fn print_status(status: &'static str, summary: UserContextSummary, json: bool) -> Result<()> {
    if json {
        print_json(&SummaryStatusOutput { status, summary })?;
    } else {
        println!(
            "User-context summary {} {} (version {}).",
            summary.id, status, summary.version
        );
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[derive(Serialize)]
struct SummaryShowOutput {
    found: bool,
    summary: Option<UserContextSummary>,
}

#[derive(Serialize)]
struct SummaryStatusOutput {
    status: &'static str,
    summary: UserContextSummary,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::query_types::{UserClaimScopeArg, UserSummaryAction},
        user_context::claims::{
            create_manual_claim, ManualClaimRequest, UserContextClaimType, UserContextSensitivity,
        },
    };

    #[test]
    fn user_summary_actions_refresh_show_sources_and_edit() -> Result<()> {
        let _dir = crate::db::test_support::ScopedTestDataDir::new("user-summary-cli-actions");
        let conn = db::open_db()?;
        create_manual_claim(
            &conn,
            &ManualClaimRequest {
                text: "Prefer summary sources",
                owner_scope: None,
                owner_key: None,
                claim_type: UserContextClaimType::Preference,
                claim_key: Some("pref:summary"),
                confidence: 1.0,
                sensitivity: UserContextSensitivity::Normal,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        drop(conn);

        run_user_summary(UserSummaryAction::Refresh {
            project: Some("/repo".to_string()),
            scope: UserClaimScopeArg::User,
            owner_key: None,
            json: true,
        })?;
        run_user_summary(UserSummaryAction::Show {
            project: Some("/repo".to_string()),
            scope: UserClaimScopeArg::User,
            owner_key: None,
            json: true,
        })?;
        run_user_summary(UserSummaryAction::Sources {
            project: Some("/repo".to_string()),
            scope: UserClaimScopeArg::User,
            owner_key: None,
            include_excluded: true,
            json: true,
        })?;
        run_user_summary(UserSummaryAction::Edit {
            project: Some("/repo".to_string()),
            scope: UserClaimScopeArg::User,
            owner_key: None,
            text: "Edited summary".to_string(),
            json: true,
        })?;

        let conn = db::open_db()?;
        let active = summary::load_active_summary(
            &conn,
            &SummaryRequest {
                owner_scope: None,
                owner_key: None,
                project: "/repo",
            },
        )?
        .ok_or_else(|| anyhow::anyhow!("summary missing"))?;
        assert_eq!(active.version, 2);
        assert_eq!(active.summary_text, "Edited summary");
        Ok(())
    }

    #[test]
    fn user_summary_json_outputs_match_subcommand_shapes() -> Result<()> {
        let summary = UserContextSummary {
            id: 1,
            user_key: "user:default".to_string(),
            owner_scope: "user".to_string(),
            owner_key: "user:default".to_string(),
            scope: "project".to_string(),
            scope_key: Some("/repo".to_string()),
            summary_text: "Profile summary".to_string(),
            source_claim_ids: vec![1],
            source_memory_ids: vec![2],
            source_activity_refs: Vec::new(),
            status: "active".to_string(),
            model: Some("deterministic-profile-v1".to_string()),
            version: 1,
            created_at_epoch: 10,
            updated_at_epoch: 10,
        };
        let show = serde_json::to_value(&SummaryShowOutput {
            found: true,
            summary: Some(summary.clone()),
        })?;
        assert!(show.get("found").is_some());
        assert!(show.get("summary").is_some());
        assert!(show.get("status").is_none());

        let status = serde_json::to_value(&SummaryStatusOutput {
            status: "refreshed",
            summary,
        })?;
        assert!(status.get("status").is_some());
        assert!(status.get("summary").is_some());
        Ok(())
    }
}
