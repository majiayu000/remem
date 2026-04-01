use std::collections::HashMap;

use crate::memory::Memory;
use crate::workstream::WorkStream;

use super::format::{format_epoch_short, format_epoch_time, format_header_datetime, type_label};
use super::types::SessionSummaryBrief;

pub(super) fn render_core_memory(output: &mut String, memories: &[Memory]) {
    let now = chrono::Utc::now().timestamp();
    let mut scored: Vec<(&Memory, f64)> = memories
        .iter()
        .map(|memory| (memory, calculate_memory_score(memory, now)))
        .collect();
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut selected = Vec::new();
    let mut total_chars = 0;
    const MAX_CHARS: usize = 3000;
    const MAX_ITEMS: usize = 6;
    const PREVIEW_LEN: usize = 200;

    for (memory, _score) in scored.iter().take(MAX_ITEMS) {
        let preview: String = memory.text.chars().take(PREVIEW_LEN).collect();
        let item_len = preview.len() + memory.title.len() + 20;
        if total_chars + item_len > MAX_CHARS && !selected.is_empty() {
            break;
        }
        selected.push((memory, preview));
        total_chars += item_len;
    }

    if selected.is_empty() {
        return;
    }

    output.push_str("## Core\n");
    for (memory, preview) in selected {
        let date = format_epoch_short(memory.updated_at_epoch);
        output.push_str(&format!(
            "**#{} {}** ({}, {})\n",
            memory.id, memory.title, memory.memory_type, date
        ));
        output.push_str(&preview);
        if memory.text.len() > PREVIEW_LEN {
            output.push_str("...");
        }
        output.push('\n');
    }
    output.push('\n');
}

pub(super) fn render_memory_index(output: &mut String, memories: &[Memory]) {
    let mut by_type: HashMap<&str, Vec<&Memory>> = HashMap::new();
    for memory in memories {
        by_type
            .entry(memory.memory_type.as_str())
            .or_default()
            .push(memory);
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
    for memory_type in &display_order {
        if let Some(memories_for_type) = by_type.get(memory_type) {
            push_memory_index_line(
                output,
                type_label(memory_type),
                memory_type,
                memories_for_type,
            );
        }
    }

    for (memory_type, memories_for_type) in &by_type {
        if !display_order.contains(memory_type) {
            push_memory_index_line(output, memory_type, memory_type, memories_for_type);
        }
    }
    output.push('\n');
}

pub(super) fn render_workstreams(output: &mut String, workstreams: &[WorkStream]) {
    output.push_str("## WorkStreams\n");
    for workstream in workstreams {
        let next = workstream.next_action.as_deref().unwrap_or("");
        let next_part = if next.is_empty() {
            String::new()
        } else {
            format!(" -> {}", next)
        };
        output.push_str(&format!(
            "- #{} [{}] {}{}\n",
            workstream.id,
            workstream.status.as_str(),
            workstream.title,
            next_part
        ));
    }
    output.push('\n');
}

pub(super) fn render_recent_sessions(output: &mut String, summaries: &[SessionSummaryBrief]) {
    output.push_str("## Sessions\n");
    for summary in summaries {
        let date = format_epoch_short(summary.created_at_epoch);
        let time = format_epoch_time(summary.created_at_epoch);
        let completed_part = summary
            .completed
            .as_deref()
            .and_then(|completed| completed.lines().find(|line| !line.trim().is_empty()))
            .map(format_completed_line)
            .unwrap_or_default();
        output.push_str(&format!(
            "- **{}** {} {}{}\n",
            date, time, summary.request, completed_part
        ));
    }
    output.push('\n');
}

pub(super) fn render_empty_state(project: &str) {
    println!(
        "# [{}] context {}\nNo previous sessions found.",
        project,
        format_header_datetime()
    );
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

fn push_memory_index_line(
    output: &mut String,
    label: &str,
    memory_type: &str,
    memories: &[&Memory],
) {
    let section_label = if label == memory_type {
        memory_type.to_string()
    } else {
        label.to_string()
    };
    output.push_str(&format!("**{}** ({}): ", section_label, memories.len()));
    let items: Vec<String> = memories
        .iter()
        .take(10)
        .map(|memory| {
            let date = format_epoch_short(memory.updated_at_epoch);
            format!("#{} {} ({})", memory.id, memory.title, date)
        })
        .collect();
    output.push_str(&items.join(" | "));
    output.push('\n');
}

fn format_completed_line(line: &str) -> String {
    let truncated: String = line.chars().take(120).collect();
    if line.len() > 120 {
        format!(" => {}...", truncated)
    } else {
        format!(" => {}", truncated)
    }
}
