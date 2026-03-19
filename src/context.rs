use anyhow::Result;
use chrono::{Local, TimeZone};
use std::collections::HashMap;

use crate::db;
use crate::memory::{self, Memory};
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
    // Load recent session summaries (what was discussed/accomplished)
    let summaries = query_recent_summaries(&conn, &project, 5).unwrap_or_default();
    // Load workstreams
    let workstreams =
        crate::workstream::query_active_workstreams(&conn, &project).unwrap_or_default();

    if memories.is_empty() && summaries.is_empty() && workstreams.is_empty() {
        render_empty_state(&project);
        timer.done("empty (no data)");
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

    // Core Memory (Tier 0): Top weighted memories with full content
    if !memories.is_empty() {
        render_core_memory(&mut output, &memories);
    }

    // Memory Index (Tier 1): All memories grouped by type
    if !memories.is_empty() {
        render_memories_by_type(&mut output, &memories);
    }

    // Active WorkStreams
    if !workstreams.is_empty() {
        render_workstreams(&mut output, &workstreams);
    }

    // Recent sessions (what was discussed/accomplished)
    if !summaries.is_empty() {
        render_recent_sessions(&mut output, &summaries);
    }

    // Footer
    output.push_str(&format!(
        "\n{} memories loaded. Use MCP search/get_observations tools to access details.\n",
        memories.len()
    ));

    print!("{}", output);

    timer.done(&format!(
        "project={} memories={} summaries={} workstreams={}",
        project,
        memories.len(),
        summaries.len(),
        workstreams.len(),
    ));
    Ok(())
}

fn calculate_memory_score(memory: &Memory, now_epoch: i64) -> f64 {
    // Type weights
    let type_weight = match memory.memory_type.as_str() {
        "decision" => 3.0,
        "bugfix" => 2.5,
        "architecture" => 2.0,
        "discovery" => 1.0,
        "preference" => 1.5,
        _ => 0.5,
    };

    // Time decay
    let age_days = (now_epoch - memory.updated_at_epoch) / 86400;
    let time_decay = if age_days <= 7 {
        1.0
    } else if age_days <= 30 {
        0.7
    } else {
        0.4
    };

    type_weight * time_decay
}

fn render_core_memory(output: &mut String, memories: &[Memory]) {
    let now = chrono::Utc::now().timestamp();

    // Calculate scores and sort
    let mut scored: Vec<(&Memory, f64)> = memories
        .iter()
        .map(|m| (m, calculate_memory_score(m, now)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top 5-8 memories, limit total to ~1500 tokens (~6000 chars)
    let mut selected = Vec::new();
    let mut total_chars = 0;
    const MAX_CHARS: usize = 6000;
    const MAX_ITEMS: usize = 8;
    const ITEM_CHAR_LIMIT: usize = 400;

    for (mem, _score) in scored.iter().take(MAX_ITEMS) {
        let truncated: String = mem.text.chars().take(ITEM_CHAR_LIMIT).collect();
        let item_len = truncated.len() + mem.title.len() + 50; // +50 for formatting
        if total_chars + item_len > MAX_CHARS && !selected.is_empty() {
            break;
        }
        selected.push((mem, truncated));
        total_chars += item_len;
    }

    if selected.is_empty() {
        return;
    }

    output.push_str("## Core Memory\n\n");
    output.push_str("Critical context loaded on every session start:\n\n");

    for (mem, truncated) in selected {
        let emoji = type_emoji(&mem.memory_type);
        let date = format_epoch_short(mem.updated_at_epoch);
        output.push_str(&format!("### {} {} (#{}, {})\n\n", emoji, mem.title, mem.id, date));
        output.push_str(&truncated);
        if mem.text.len() > ITEM_CHAR_LIMIT {
            output.push_str("...");
        }
        output.push_str("\n\n");
    }
}

fn render_memories_by_type(output: &mut String, memories: &[Memory]) {
    output.push_str("## Memory Index\n\n");

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

/// A recent session summary for display.
struct SessionSummaryBrief {
    request: String,
    completed: Option<String>,
    created_at_epoch: i64,
}

fn query_recent_summaries(
    conn: &rusqlite::Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<SessionSummaryBrief>> {
    let mut stmt = conn.prepare(
        "SELECT request, completed, created_at_epoch \
         FROM session_summaries \
         WHERE project = ?1 AND request IS NOT NULL AND request != '' \
         ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![project, limit as i64],
        |row| {
            Ok(SessionSummaryBrief {
                request: row.get(0)?,
                completed: row.get(1)?,
                created_at_epoch: row.get(2)?,
            })
        },
    )?;
    let mut results = Vec::new();
    for row in rows {
        if let Ok(r) = row {
            results.push(r);
        }
    }
    Ok(results)
}

fn render_recent_sessions(output: &mut String, summaries: &[SessionSummaryBrief]) {
    output.push_str("## Recent Sessions\n\n");

    for s in summaries {
        let date = format_epoch_short(s.created_at_epoch);
        let time = format_epoch_time(s.created_at_epoch);

        // Request: what the user asked
        output.push_str(&format!("**{}** {} — {}\n", date, time, s.request));

        // Completed: first bullet point only (keep it concise)
        if let Some(completed) = &s.completed {
            let first_line = completed
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            let truncated: String = first_line.chars().take(200).collect();
            if !truncated.is_empty() {
                output.push_str(&format!("  {}\n", truncated));
            }
        }
        output.push('\n');
    }
}

fn render_empty_state(project: &str) {
    println!(
        "# [{}] recent context, {}\n\nNo previous sessions found for this project yet.",
        project,
        format_header_datetime()
    );
}
