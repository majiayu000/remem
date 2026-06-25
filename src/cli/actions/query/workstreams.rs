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
    let results = workstream::query_workstreams(&conn, project, status_str)?;
    if json {
        let output = WorkstreamListJson {
            project: project.to_string(),
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
        output.push_str(&format!(
            "#{} [{}] {}\n",
            item.id,
            item.status.as_str(),
            item.title
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
