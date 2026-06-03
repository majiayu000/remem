use anyhow::Result;

use super::why::resolve_current_project;
use crate::db;

/// `remem trace <topic_key>` — print the time-ordered trace of one topic
/// across sessions (Trace Weaver, read-only). Phase 2.
pub(in crate::cli) fn run_trace(topic_key: &str, project: Option<&str>) -> Result<()> {
    let conn = db::open_db()?;
    let Some(project) = resolve_current_project(project)? else {
        println!("No project resolved; pass --project <path>.");
        return Ok(());
    };
    let trace = db::load_trace_by_topic_key(&conn, &project, topic_key)?;
    if trace.is_empty() {
        println!("No topic segments for '{topic_key}' in {project}.");
        return Ok(());
    }
    println!(
        "Topic trace '{topic_key}' — {} segment(s) in {project}",
        trace.len()
    );
    for seg in &trace {
        println!(
            "  [{status}] events {from}..{to}  {title}",
            status = seg.status,
            from = seg.covered_from_event_id,
            to = seg.covered_to_event_id,
            title = seg.title,
        );
        if !seg.summary.is_empty() {
            println!("      {}", seg.summary);
        }
    }
    Ok(())
}
