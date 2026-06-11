use anyhow::{bail, Result};
use serde::Serialize;

use crate::cli::types::TimelineAction;
use crate::{db, retrieval::search::search_observations};

use super::show::format_memory_timestamp;

pub(in crate::cli) fn run_timeline(action: TimelineAction) -> Result<()> {
    match action {
        TimelineAction::Around {
            anchor,
            query,
            project,
            depth_before,
            depth_after,
            json,
        } => run_timeline_around(
            anchor,
            query.as_deref(),
            project.as_deref(),
            depth_before,
            depth_after,
            json,
        ),
        TimelineAction::Report {
            project,
            full,
            json,
        } => run_timeline_report(&project, full, json),
    }
}

fn run_timeline_around(
    anchor: Option<i64>,
    query: Option<&str>,
    project: Option<&str>,
    depth_before: i64,
    depth_after: i64,
    json: bool,
) -> Result<()> {
    let conn = db::open_db()?;
    let anchor_id = resolve_anchor(&conn, anchor, query, project)?;
    let before = depth_before.max(0);
    let after = depth_after.max(0);
    let results = db::get_timeline_around(&conn, anchor_id, before, after, project)?;
    if json {
        let output = TimelineAroundJson {
            anchor_id,
            query: query.map(str::to_string),
            project: project.map(str::to_string),
            depth_before: before,
            depth_after: after,
            count: results.len(),
            results,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    print!("{}", render_timeline_around(anchor_id, &results));
    Ok(())
}

fn resolve_anchor(
    conn: &rusqlite::Connection,
    anchor: Option<i64>,
    query: Option<&str>,
    project: Option<&str>,
) -> Result<i64> {
    if let Some(anchor) = anchor {
        let found = db::get_observations_by_ids(conn, &[anchor], project)?;
        if found.is_empty() {
            bail!("No observation found for anchor id {anchor}");
        }
        return Ok(anchor);
    }
    let Some(query) = query else {
        bail!("timeline around requires --anchor or --query");
    };
    let results = search_observations(conn, Some(query), project, None, 1, 0, true)?;
    let Some(result) = results.first() else {
        bail!("No results for query '{query}'");
    };
    Ok(result.id)
}

fn run_timeline_report(project: &str, full: bool, json: bool) -> Result<()> {
    let conn = db::open_db()?;
    if json {
        let output = TimelineReportJson {
            project: project.to_string(),
            full,
            report: crate::timeline::generate_timeline_report_data(&conn, project, full)?,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    let report = crate::timeline::generate_timeline_report(&conn, project, full)?;
    print!("{report}");
    Ok(())
}

fn render_timeline_around(anchor_id: i64, results: &[db::Observation]) -> String {
    let mut output = format!("Timeline around observation #{anchor_id}:\n\n");
    for observation in results {
        let marker = if observation.id == anchor_id {
            "*"
        } else {
            " "
        };
        let title = observation.title.as_deref().unwrap_or("(untitled)");
        output.push_str(&format!(
            "{} #{} [{}] {} {}\n",
            marker,
            observation.id,
            observation.r#type,
            format_memory_timestamp(observation.created_at_epoch),
            title
        ));
    }
    output
}

#[derive(Debug, Serialize)]
struct TimelineAroundJson {
    anchor_id: i64,
    query: Option<String>,
    project: Option<String>,
    depth_before: i64,
    depth_after: i64,
    count: usize,
    results: Vec<db::Observation>,
}

#[derive(Debug, Serialize)]
struct TimelineReportJson {
    project: String,
    full: bool,
    report: crate::timeline::TimelineReportData,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn query_anchor_resolves_observation_id_not_memory_id() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;

        let memory_id = crate::memory::insert_memory(
            &conn,
            Some("m1"),
            "/repo",
            None,
            "Release manifest memory",
            "release manifest from memory",
            "decision",
            None,
        )?;
        let unrelated_observation_id = crate::db::insert_observation(
            &conn,
            "s0",
            "/repo",
            "decision",
            Some("Unrelated observation"),
            None,
            Some("unrelated operational note"),
            None,
            None,
            None,
            None,
            None,
            0,
        )?;
        let matching_observation_id = crate::db::insert_observation(
            &conn,
            "s1",
            "/repo",
            "decision",
            Some("Release manifest observation"),
            None,
            Some("release manifest observation anchor"),
            None,
            None,
            None,
            None,
            None,
            0,
        )?;

        assert_eq!(memory_id, unrelated_observation_id);
        assert_ne!(memory_id, matching_observation_id);

        let resolved = resolve_anchor(&conn, None, Some("release manifest"), Some("/repo"))?;
        assert_eq!(resolved, matching_observation_id);
        Ok(())
    }
}
