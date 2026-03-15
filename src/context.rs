use anyhow::Result;
use chrono::{Local, TimeZone};
use std::collections::HashMap;

use crate::db;
use crate::memory::{self, Event, Memory};
use crate::workstream::WorkStream;

use crate::db::project_from_cwd;

fn format_header_datetime() -> String {
    Local::now().format("%Y-%m-%d %-I:%M%P %:z").to_string()
}

fn type_emoji(t: &str) -> &'static str {
    match t {
        "decision" => "\u{1f535}",
        "bugfix" => "\u{1f41b}",
        "architecture" => "\u{2728}",
        "discovery" => "\u{1f50d}",
        "preference" => "\u{2699}\u{fe0f}",
        "session_activity" => "\u{1f4cb}",
        _ => "\u{25cf}",
    }
}

fn type_label(t: &str) -> &'static str {
    match t {
        "decision" => "Architecture Decisions",
        "bugfix" => "Bug Fixes",
        "architecture" => "Architecture",
        "discovery" => "Discoveries",
        "preference" => "Preferences",
        "session_activity" => "Session Activity",
        _ => "Other",
    }
}

fn format_epoch_short(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|dt| dt.format("%m-%d").to_string())
        .unwrap_or_default()
}

fn format_epoch_time(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|dt| dt.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}

pub fn generate_context(cwd: &str, _session_id: Option<&str>, _use_colors: bool) -> Result<()> {
    let timer = crate::log::Timer::start("context", &format!("cwd={}", cwd));
    let project = project_from_cwd(cwd);

    let conn = match db::open_db() {
        Ok(c) => c,
        Err(e) => {
            crate::log::warn(
                "context",
                &format!("open_db failed for project={}: {}", project, e),
            );
            render_empty_state(&project);
            timer.done("empty (no DB)");
            return Ok(());
        }
    };

    // Load memories grouped by type
    let memories = memory::get_recent_memories(&conn, &project, 50).unwrap_or_default();
    // Load recent events for activity summary
    let events = memory::get_recent_events(&conn, &project, 30).unwrap_or_default();
    // Load workstreams
    let workstreams =
        crate::workstream::query_active_workstreams(&conn, &project).unwrap_or_default();

    if memories.is_empty() && events.is_empty() && workstreams.is_empty() {
        // Fall back to legacy observations if no new data yet
        render_legacy_context(&conn, &project, cwd)?;
        return Ok(());
    }

    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "# [{}] recent context, {}\n\n",
        project,
        format_header_datetime()
    ));

    // Reminder
    output.push_str(
        "**\u{63d0}\u{793a}\u{ff1a}** \u{4fee}\u{6539}\u{5df2}\u{77e5}\u{9879}\u{76ee}\u{4ee3}\u{7801}\u{524d}\u{ff0c}\u{5148}\u{7528} remem search \u{5de5}\u{5177}\u{67e5}\u{8be2}\u{76f8}\u{5173}\u{8bb0}\u{5fc6}\u{3002}\u{505a}\u{51fa}\u{91cd}\u{8981}\u{51b3}\u{7b56}\u{6216}\u{4fee}\u{590d} bug \u{540e}\u{ff0c}\u{7528} save_memory(type=..., topic_key=...) \u{8bb0}\u{5f55}\u{3002}\n\n",
    );

    // Key memories grouped by type
    if !memories.is_empty() {
        render_memories_by_type(&mut output, &memories);
    }

    // Active WorkStreams
    if !workstreams.is_empty() {
        render_workstreams(&mut output, &workstreams);
    }

    // Recent activity from events
    if !events.is_empty() {
        render_recent_activity(&mut output, &events);
    }

    // Footer
    output.push_str(&format!(
        "\n{} memories loaded. Use MCP search/get_observations tools to access details.\n",
        memories.len()
    ));

    print!("{}", output);

    timer.done(&format!(
        "project={} memories={} events={} workstreams={}",
        project,
        memories.len(),
        events.len(),
        workstreams.len(),
    ));
    Ok(())
}

fn render_memories_by_type(output: &mut String, memories: &[Memory]) {
    // Group by type
    let mut by_type: HashMap<&str, Vec<&Memory>> = HashMap::new();
    for m in memories {
        by_type.entry(m.memory_type.as_str()).or_default().push(m);
    }

    // Display order: decision, bugfix, architecture, discovery, preference, session_activity
    let display_order = [
        "decision",
        "bugfix",
        "architecture",
        "discovery",
        "preference",
        "session_activity",
    ];

    for mem_type in &display_order {
        if let Some(mems) = by_type.get(mem_type) {
            let emoji = type_emoji(mem_type);
            let label = type_label(mem_type);
            output.push_str(&format!("### {} {} ({})\n", emoji, label, mems.len()));
            output.push_str("| # | Title | Updated |\n");
            output.push_str("|---|-------|---------|\n");
            for m in mems.iter().take(10) {
                let date = format_epoch_short(m.updated_at_epoch);
                output.push_str(&format!("| {} | {} | {} |\n", m.id, m.title, date));
            }
            output.push('\n');
        }
    }

    // Any types not in display_order
    for (mem_type, mems) in &by_type {
        if !display_order.contains(mem_type) {
            let emoji = type_emoji(mem_type);
            output.push_str(&format!("### {} {} ({})\n", emoji, mem_type, mems.len()));
            output.push_str("| # | Title | Updated |\n");
            output.push_str("|---|-------|---------|\n");
            for m in mems.iter().take(10) {
                let date = format_epoch_short(m.updated_at_epoch);
                output.push_str(&format!("| {} | {} | {} |\n", m.id, m.title, date));
            }
            output.push('\n');
        }
    }
}

fn render_workstreams(output: &mut String, workstreams: &[WorkStream]) {
    output.push_str("### Active WorkStreams\n");
    output.push_str("| # | Status | WorkStream | Progress | Next Action |\n");
    output.push_str("|---|--------|------------|----------|-------------|\n");
    for ws in workstreams {
        let status = ws.status.as_str();
        let progress = ws.progress.as_deref().unwrap_or("-");
        let next = ws.next_action.as_deref().unwrap_or("-");
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            ws.id, status, ws.title, progress, next
        ));
    }
    output.push('\n');
}

fn render_recent_activity(output: &mut String, events: &[Event]) {
    output.push_str("### Recent Activity\n");

    // Compact: group consecutive same-type events
    let mut file_edits: HashMap<String, usize> = HashMap::new();
    let mut other_events: Vec<&Event> = Vec::new();

    for e in events {
        if e.event_type == "file_edit" || e.event_type == "file_create" {
            if let Some(files_json) = &e.files {
                if let Ok(files) = serde_json::from_str::<Vec<String>>(files_json) {
                    for f in files {
                        // Shorten path to last 2 components
                        let short = crate::observe::short_path(&f);
                        *file_edits.entry(short.to_string()).or_insert(0) += 1;
                    }
                }
            }
        } else {
            other_events.push(e);
        }
    }

    // Show file edits summary
    if !file_edits.is_empty() {
        let mut sorted: Vec<_> = file_edits.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        output.push_str("Files modified: ");
        let items: Vec<String> = sorted
            .iter()
            .take(10)
            .map(|(f, count)| {
                if *count > 1 {
                    format!("{} (\u{00d7}{})", f, count)
                } else {
                    f.clone()
                }
            })
            .collect();
        output.push_str(&items.join(", "));
        output.push('\n');
    }

    // Show other events (bash, agent, search)
    for e in other_events.iter().take(10) {
        let time = format_epoch_time(e.created_at_epoch);
        output.push_str(&format!("- {} {}\n", time, e.summary));
    }
    output.push('\n');
}

/// Fall back to legacy observation-based context when no memories exist yet.
fn render_legacy_context(conn: &rusqlite::Connection, project: &str, _cwd: &str) -> Result<()> {
    use crate::db_models::OBSERVATION_TYPES;

    let type_refs: Vec<&str> = OBSERVATION_TYPES.to_vec();
    let observations = crate::db_query::query_observations(conn, project, &type_refs, 50)?;

    if observations.is_empty() {
        render_empty_state(project);
        crate::log::info("context", "empty (no data, legacy)");
        return Ok(());
    }

    let mut output = String::new();
    output.push_str(&format!(
        "# [{}] recent context, {}\n\n",
        project,
        format_header_datetime()
    ));
    output.push_str(
        "**\u{63d0}\u{793a}\u{ff1a}** \u{4fee}\u{6539}\u{5df2}\u{77e5}\u{9879}\u{76ee}\u{4ee3}\u{7801}\u{524d}\u{ff0c}\u{5148}\u{7528} remem search \u{5de5}\u{5177}\u{67e5}\u{8be2}\u{76f8}\u{5173}\u{8bb0}\u{5fc6}\u{3002}\n\n",
    );

    // Simple observation listing
    output.push_str("### Observations (legacy)\n");
    output.push_str("| # | Type | Title |\n");
    output.push_str("|---|------|-------|\n");
    for obs in observations.iter().take(30) {
        let title = obs.title.as_deref().unwrap_or("-");
        output.push_str(&format!("| {} | {} | {} |\n", obs.id, obs.r#type, title));
    }
    output.push('\n');

    output.push_str(&format!(
        "\n{} observations loaded. Use MCP search tools to access details.\n",
        observations.len()
    ));

    print!("{}", output);
    crate::log::info(
        "context",
        &format!("project={} obs={} (legacy)", project, observations.len()),
    );
    Ok(())
}

fn render_empty_state(project: &str) {
    println!(
        "# [{}] recent context, {}\n\nNo previous sessions found for this project yet.",
        project,
        format_header_datetime()
    );
}
