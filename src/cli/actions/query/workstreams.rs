use anyhow::{bail, Result};
use serde::Serialize;

use crate::cli::types::{WorkstreamAction, WorkstreamStatusArg};
use crate::{db, workstream};

pub(in crate::cli) fn run_workstreams(action: WorkstreamAction) -> Result<()> {
    match action {
        WorkstreamAction::List {
            project,
            status,
            json,
        } => run_workstream_list(&project, status, json),
        WorkstreamAction::Update {
            id,
            project,
            status,
            next_action,
            blockers,
            confirm,
            json,
        } => run_workstream_update(
            id,
            &project,
            status,
            next_action.as_deref(),
            blockers.as_deref(),
            confirm,
            json,
        ),
        WorkstreamAction::Merge {
            project,
            into,
            duplicates,
            confirm,
            json,
        } => run_workstream_merge(&project, into, &duplicates, confirm, json),
    }
}

fn run_workstream_list(
    project: &str,
    status: Option<WorkstreamStatusArg>,
    json: bool,
) -> Result<()> {
    let conn = db::open_db()?;
    let status_str = status.map(WorkstreamStatusArg::as_str);
    let results = workstream::query_workstreams(&conn, project, status_str)?
        .into_iter()
        .map(|item| {
            workstream::redact_workstream_for_output(
                item,
                crate::adapter::common::redact_projected_sensitive_text,
                crate::adapter::common::redact_projected_project_text,
            )
        })
        .collect::<Vec<_>>();
    if json {
        let output = WorkstreamListJson {
            // Envelope project comes from --project; redact so list JSON cannot
            // echo a credential-bearing argument while item.project is sanitized.
            project: crate::adapter::common::redact_projected_project_text(project),
            status: status_str.map(str::to_string),
            count: results.len(),
            workstreams: results,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    print!("{}", render_workstream_list(&results));
    Ok(())
}

fn run_workstream_update(
    id: i64,
    project: &str,
    status: Option<WorkstreamStatusArg>,
    next_action: Option<&str>,
    blockers: Option<&str>,
    confirm: bool,
    json: bool,
) -> Result<()> {
    validate_workstream_update_request(status, next_action, blockers, confirm)?;
    let conn = db::open_db()?;
    let belongs_to_project = workstream::query_workstreams(&conn, project, None)?
        .iter()
        .any(|item| item.id == id);
    if !belongs_to_project {
        bail!("No workstream found for id {id} in project {project}");
    }
    let updated = workstream::update_workstream_manual(
        &conn,
        id,
        status.map(WorkstreamStatusArg::as_str),
        next_action,
        blockers,
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&WorkstreamUpdateJson {
                id,
                project,
                updated
            })?
        );
        return Ok(());
    }
    if updated {
        println!("Updated workstream #{id}.");
    } else {
        println!("No workstream found for id {id}.");
    }
    Ok(())
}

fn validate_workstream_update_request(
    status: Option<WorkstreamStatusArg>,
    next_action: Option<&str>,
    blockers: Option<&str>,
    confirm: bool,
) -> Result<()> {
    if status.is_none() && next_action.is_none() && blockers.is_none() {
        bail!("workstreams update requires --status, --next-action, or --blockers");
    }
    if !confirm {
        bail!("workstreams update requires --confirm");
    }
    Ok(())
}

fn run_workstream_merge(
    project: &str,
    canonical_id: i64,
    duplicate_ids: &[i64],
    confirm: bool,
    json: bool,
) -> Result<()> {
    validate_workstream_merge_request(canonical_id, duplicate_ids, confirm)?;
    let conn = db::open_db()?;
    let result = workstream::merge_workstreams_manual(&conn, project, canonical_id, duplicate_ids)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&WorkstreamMergeJson { project, result })?
        );
        return Ok(());
    }
    println!(
        "Merged {} duplicate workstream(s) into workstream #{}.",
        duplicate_ids.len(),
        canonical_id
    );
    Ok(())
}

fn validate_workstream_merge_request(
    canonical_id: i64,
    duplicate_ids: &[i64],
    confirm: bool,
) -> Result<()> {
    if duplicate_ids.is_empty() {
        bail!("workstreams merge requires at least one duplicate id");
    }
    if duplicate_ids.contains(&canonical_id) {
        bail!("workstreams merge cannot merge a workstream into itself");
    }
    if !confirm {
        bail!("workstreams merge requires --confirm");
    }
    Ok(())
}

fn render_workstream_list(workstreams: &[workstream::WorkStream]) -> String {
    let mut output = String::new();
    if workstreams.is_empty() {
        output.push_str("No workstreams found.\n");
        return output;
    }
    output.push_str("Workstreams:\n\n");
    for item in workstreams {
        let heading = item.display_label.as_deref().unwrap_or(&item.title);
        output.push_str(&format!(
            "#{} [{}] {}\n",
            item.id,
            item.status.as_str(),
            heading
        ));
        if let Some(next_action) = &item.next_action {
            output.push_str(&format!("  next: {next_action}\n"));
        }
        if let Some(blockers) = &item.blockers {
            output.push_str(&format!("  blockers: {blockers}\n"));
        }
        output.push('\n');
    }
    output
}

#[derive(Debug, Serialize)]
struct WorkstreamListJson {
    project: String,
    status: Option<String>,
    count: usize,
    workstreams: Vec<workstream::WorkStream>,
}

#[derive(Debug, Serialize)]
struct WorkstreamUpdateJson<'a> {
    id: i64,
    project: &'a str,
    updated: bool,
}

#[derive(Debug, Serialize)]
struct WorkstreamMergeJson<'a> {
    project: &'a str,
    result: workstream::WorkStreamMergeResult,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workstream_update_rejects_empty_mutation() {
        let error = validate_workstream_update_request(None, None, None, true).unwrap_err();
        assert!(error.to_string().contains("--status"));
    }

    #[test]
    fn list_projection_redacts_secret_bearing_fields() {
        let workstreams = vec![workstream::redact_workstream_for_output(
            workstream::WorkStream {
                id: 7,
                project: "test/proj".to_string(),
                title: "Safe listing".to_string(),
                description: Some("token=desc-secret".to_string()),
                status: workstream::WorkStreamStatus::Active,
                progress: Some("token=progress-secret".to_string()),
                next_action: Some("token=next-secret".to_string()),
                blockers: Some("token=blocker-secret".to_string()),
                created_at_epoch: 1735660800,
                updated_at_epoch: 1735660800,
                completed_at_epoch: None,
                mmdd: None,
                session_intent: Some("fix".to_string()),
                session_topic: Some("token=mcp-cli-workstream-secret".to_string()),
                display_label: None,
                session_intent_source: Some("summary".to_string()),
            },
            crate::adapter::common::redact_projected_sensitive_text,
            crate::adapter::common::redact_projected_project_text,
        )];
        let rendered = render_workstream_list(&workstreams);
        assert!(
            !rendered.contains("mcp-cli-workstream-secret"),
            "{rendered}"
        );
        assert!(rendered.contains("token=[REDACTED]"), "{rendered}");
        let encoded = serde_json::to_string(&workstreams).unwrap();
        assert!(!encoded.contains("desc-secret"), "{encoded}");
        assert!(!encoded.contains("progress-secret"), "{encoded}");
        assert!(!encoded.contains("next-secret"), "{encoded}");
        assert!(!encoded.contains("blocker-secret"), "{encoded}");
    }

    #[test]
    fn list_projection_redacts_short_inline_credential_assignments() {
        let workstreams = vec![workstream::redact_workstream_for_output(
            workstream::WorkStream {
                id: 8,
                project: "test/proj".to_string(),
                title: "Investigate token=abc123".to_string(),
                description: Some("Fix OAuth token=short-secret".to_string()),
                status: workstream::WorkStreamStatus::Active,
                progress: None,
                next_action: Some("Rotate token=xyz789".to_string()),
                blockers: None,
                created_at_epoch: 1735660800,
                updated_at_epoch: 1735660800,
                completed_at_epoch: None,
                mmdd: None,
                session_intent: Some("FIX".to_string()),
                session_topic: Some("Investigate token=abc123".to_string()),
                display_label: None,
                session_intent_source: Some("summary".to_string()),
            },
            crate::adapter::common::redact_projected_sensitive_text,
            crate::adapter::common::redact_projected_project_text,
        )];
        let encoded = serde_json::to_string(&workstreams).unwrap();
        assert!(!encoded.contains("abc123"), "{encoded}");
        assert!(!encoded.contains("short-secret"), "{encoded}");
        assert!(!encoded.contains("xyz789"), "{encoded}");
        assert!(encoded.contains("token=[REDACTED]"), "{encoded}");
    }

    #[test]
    fn list_json_envelope_redacts_secret_bearing_project() {
        let project = "token=envelope-project-secret";
        let output = WorkstreamListJson {
            project: crate::adapter::common::redact_projected_project_text(project),
            status: None,
            count: 0,
            workstreams: vec![],
        };
        let encoded = serde_json::to_string(&output).unwrap();
        assert!(!encoded.contains("envelope-project-secret"), "{encoded}");
        assert!(encoded.contains("token=[REDACTED]"), "{encoded}");
    }

    #[test]
    fn list_projection_redacts_space_separated_credential_options() {
        let workstreams = vec![workstream::redact_workstream_for_output(
            workstream::WorkStream {
                id: 9,
                project: "test/proj".to_string(),
                title: "Safe listing".to_string(),
                description: None,
                status: workstream::WorkStreamStatus::Active,
                progress: Some("Run curl --oauth2-bearer tiny-token".to_string()),
                next_action: Some("Retry with -u alice:pw".to_string()),
                blockers: None,
                created_at_epoch: 1735660800,
                updated_at_epoch: 1735660800,
                completed_at_epoch: None,
                mmdd: None,
                session_intent: Some("FIX".to_string()),
                session_topic: Some("Command remediation".to_string()),
                display_label: None,
                session_intent_source: Some("summary".to_string()),
            },
            crate::adapter::common::redact_projected_sensitive_text,
            crate::adapter::common::redact_projected_project_text,
        )];
        let encoded = serde_json::to_string(&workstreams).unwrap();
        assert!(!encoded.contains("tiny-token"), "{encoded}");
        assert!(!encoded.contains("alice:pw"), "{encoded}");
        assert!(encoded.contains("--oauth2-bearer [REDACTED]"), "{encoded}");
        assert!(encoded.contains("-u [REDACTED]"), "{encoded}");
    }

    #[test]
    fn list_projection_preserves_benign_long_project_paths() {
        let project = "/home/u/project2abcd1234567890abcdef12";
        let workstreams = vec![workstream::redact_workstream_for_output(
            workstream::WorkStream {
                id: 10,
                project: project.to_string(),
                title: "Path listing".to_string(),
                description: None,
                status: workstream::WorkStreamStatus::Active,
                progress: None,
                next_action: None,
                blockers: None,
                created_at_epoch: 1735660800,
                updated_at_epoch: 1735660800,
                completed_at_epoch: None,
                mmdd: None,
                session_intent: Some("FIX".to_string()),
                session_topic: Some("Path listing".to_string()),
                display_label: None,
                session_intent_source: Some("summary".to_string()),
            },
            crate::adapter::common::redact_projected_sensitive_text,
            crate::adapter::common::redact_projected_project_text,
        )];
        let encoded = serde_json::to_string(&workstreams).unwrap();
        assert!(encoded.contains(project), "{encoded}");
        assert!(!encoded.contains("[REDACTED]"), "{encoded}");
    }

    #[test]
    fn workstream_update_requires_confirmation() {
        let error = validate_workstream_update_request(
            Some(WorkstreamStatusArg::Paused),
            None,
            None,
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("--confirm"));
    }

    #[test]
    fn workstream_merge_requires_confirmation() {
        let error = validate_workstream_merge_request(1, &[2], false).unwrap_err();
        assert!(error.to_string().contains("--confirm"));
    }

    #[test]
    fn workstream_merge_rejects_self_merge() {
        let error = validate_workstream_merge_request(1, &[2, 1], true).unwrap_err();
        assert!(error.to_string().contains("itself"));
    }
}
