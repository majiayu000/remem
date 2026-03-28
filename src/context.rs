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

fn type_label(t: &str) -> &'static str {
    match t {
        "decision" => "Decisions",
        "bugfix" => "Bug Fixes",
        "architecture" => "Architecture",
        "discovery" => "Discoveries",
        "preference" => "Preferences",
        "session_activity" => "Sessions",
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
        .map(|dt| dt.format("%-I:%M%P").to_string())
        .unwrap_or_default()
}

pub fn generate_context(cwd: &str, _session_id: Option<&str>, _use_colors: bool) -> Result<()> {
    let timer = crate::log::Timer::start("context", &format!("cwd={}", cwd));
    let project = project_from_cwd(cwd);
    let current_branch = db::detect_git_branch(cwd);

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

    // Smart context: search by project name for relevance, then fill with recent
    let mut memories = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    // Layer 1: Search using project name as query (finds project-relevant memories)
    let project_query = project.rsplit('/').next().unwrap_or(&project);
    if let Ok(searched) = crate::search::search(
        &conn,
        Some(project_query),
        Some(&project),
        None,
        20,
        0,
        false,
    ) {
        for m in searched {
            seen_ids.insert(m.id);
            memories.push(m);
        }
    }

    // Layer 2: Fill with recent memories (time-based, catches things search missed)
    let recent = memory::get_recent_memories(&conn, &project, 50).unwrap_or_default();
    for m in recent {
        if seen_ids.insert(m.id) {
            memories.push(m);
        }
    }

    memories.truncate(50);

    // Sort: current branch first, then branchless, then main, then others
    if let Some(ref branch) = current_branch {
        memories.sort_by(|a, b| {
            let score = |m: &Memory| -> u8 {
                match &m.branch {
                    Some(br) if br == branch => 0,
                    None => 1,
                    Some(br) if br == "main" || br == "master" => 2,
                    _ => 3,
                }
            };
            score(a).cmp(&score(b))
        });
    }
    let summaries = query_recent_summaries(&conn, &project, 5).unwrap_or_default();
    let workstreams =
        crate::workstream::query_active_workstreams(&conn, &project).unwrap_or_default();

    if memories.is_empty() && summaries.is_empty() && workstreams.is_empty() {
        render_empty_state(&project);
        timer.done("empty (no data)");
        return Ok(());
    }

    let mut output = String::new();

    let branch_label = current_branch
        .as_deref()
        .map(|b| format!(" @{}", b))
        .unwrap_or_default();
    output.push_str(&format!(
        "# [{}{}] context {}\n",
        project,
        branch_label,
        format_header_datetime()
    ));
    output.push_str(
        "Use `search`/`get_observations` for details. `save_memory` after decisions/bugfixes.\n\n",
    );

    // Preferences section — top priority, rendered before core memories
    if let Err(e) = crate::preference::render_preferences(&mut output, &conn, &project, cwd) {
        crate::log::warn("context", &format!("render_preferences failed: {}", e));
    }

    if !memories.is_empty() {
        render_core_memory(&mut output, &memories);
    }

    if !memories.is_empty() {
        render_memory_index(&mut output, &memories);
    }

    if !workstreams.is_empty() {
        render_workstreams(&mut output, &workstreams);
    }

    if !summaries.is_empty() {
        render_recent_sessions(&mut output, &summaries);
    }

    output.push_str(&format!("{} memories loaded.\n", memories.len()));

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
    let type_weight = match memory.memory_type.as_str() {
        "decision" => 3.0,
        "bugfix" => 2.5,
        "architecture" => 2.0,
        "discovery" => 1.0,
        "preference" => 1.5,
        _ => 0.5,
    };

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

/// Core Memory: top scored memories with truncated preview (200 chars max).
fn render_core_memory(output: &mut String, memories: &[Memory]) {
    let now = chrono::Utc::now().timestamp();

    let mut scored: Vec<(&Memory, f64)> = memories
        .iter()
        .map(|m| (m, calculate_memory_score(m, now)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected = Vec::new();
    let mut total_chars = 0;
    const MAX_CHARS: usize = 3000;
    const MAX_ITEMS: usize = 6;
    const PREVIEW_LEN: usize = 200;

    for (mem, _score) in scored.iter().take(MAX_ITEMS) {
        let preview: String = mem.text.chars().take(PREVIEW_LEN).collect();
        let item_len = preview.len() + mem.title.len() + 20;
        if total_chars + item_len > MAX_CHARS && !selected.is_empty() {
            break;
        }
        selected.push((mem, preview));
        total_chars += item_len;
    }

    if selected.is_empty() {
        return;
    }

    output.push_str("## Core\n");
    for (mem, preview) in selected {
        let date = format_epoch_short(mem.updated_at_epoch);
        output.push_str(&format!(
            "**#{} {}** ({}, {})\n",
            mem.id, mem.title, mem.memory_type, date
        ));
        output.push_str(&preview);
        if mem.text.len() > PREVIEW_LEN {
            output.push_str("...");
        }
        output.push('\n');
    }
    output.push('\n');
}

/// Memory Index: compact list grouped by type.
fn render_memory_index(output: &mut String, memories: &[Memory]) {
    let mut by_type: HashMap<&str, Vec<&Memory>> = HashMap::new();
    for m in memories {
        by_type.entry(m.memory_type.as_str()).or_default().push(m);
    }

    let display_order = [
        "decision",
        "bugfix",
        "architecture",
        "discovery",
        "preference",
        "session_activity",
    ];

    output.push_str("## Index\n");

    for mem_type in &display_order {
        if let Some(mems) = by_type.get(mem_type) {
            let label = type_label(mem_type);
            output.push_str(&format!("**{}** ({}): ", label, mems.len()));
            let items: Vec<String> = mems
                .iter()
                .take(10)
                .map(|m| {
                    let date = format_epoch_short(m.updated_at_epoch);
                    format!("#{} {} ({})", m.id, m.title, date)
                })
                .collect();
            output.push_str(&items.join(" | "));
            output.push('\n');
        }
    }

    // Types not in display_order
    for (mem_type, mems) in &by_type {
        if !display_order.contains(mem_type) {
            output.push_str(&format!("**{}** ({}): ", mem_type, mems.len()));
            let items: Vec<String> = mems
                .iter()
                .take(10)
                .map(|m| {
                    let date = format_epoch_short(m.updated_at_epoch);
                    format!("#{} {} ({})", m.id, m.title, date)
                })
                .collect();
            output.push_str(&items.join(" | "));
            output.push('\n');
        }
    }
    output.push('\n');
}

fn render_workstreams(output: &mut String, workstreams: &[WorkStream]) {
    output.push_str("## WorkStreams\n");
    for ws in workstreams {
        let status = ws.status.as_str();
        let next = ws.next_action.as_deref().unwrap_or("");
        let next_part = if next.is_empty() {
            String::new()
        } else {
            format!(" -> {}", next)
        };
        output.push_str(&format!(
            "- #{} [{}] {}{}\n",
            ws.id, status, ws.title, next_part
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
    let rows = stmt.query_map(rusqlite::params![project, limit as i64], |row| {
        Ok(SessionSummaryBrief {
            request: row.get(0)?,
            completed: row.get(1)?,
            created_at_epoch: row.get(2)?,
        })
    })?;
    Ok(rows.flatten().collect())
}

fn render_recent_sessions(output: &mut String, summaries: &[SessionSummaryBrief]) {
    output.push_str("## Sessions\n");
    for s in summaries {
        let date = format_epoch_short(s.created_at_epoch);
        let time = format_epoch_time(s.created_at_epoch);
        // Single-line: date time — request [completed first line if available]
        let completed_part = s
            .completed
            .as_deref()
            .and_then(|c| c.lines().find(|l| !l.trim().is_empty()))
            .map(|line| {
                let truncated: String = line.chars().take(120).collect();
                if line.len() > 120 {
                    format!(" => {}...", truncated)
                } else {
                    format!(" => {}", truncated)
                }
            })
            .unwrap_or_default();
        output.push_str(&format!(
            "- **{}** {} {}{}\n",
            date, time, s.request, completed_part
        ));
    }
    output.push('\n');
}

fn render_empty_state(project: &str) {
    println!(
        "# [{}] context {}\nNo previous sessions found.",
        project,
        format_header_datetime()
    );
}
