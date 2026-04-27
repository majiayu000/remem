use crate::memory::Memory;
use std::collections::HashMap;

use super::super::format::format_epoch_short;
use super::super::memory_traits::is_memory_self_diagnostic;

const MAX_CHARS: usize = 3000;
const MAX_ITEMS: usize = 6;
const PREVIEW_LEN: usize = 200;
const MAX_PRIMARY_ITEMS_PER_TYPE: usize = 2;
const MIN_ADDITIONAL_CORE_SCORE: f64 = 1.3;

pub(in crate::context) fn render_core_memory(output: &mut String, memories: &[Memory]) {
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

    let mut selected: Vec<(&Memory, String)> = Vec::new();
    let mut total_chars = 0;
    let mut selected_ids = std::collections::HashSet::new();
    let mut type_counts: HashMap<&str, usize> = HashMap::new();

    for (memory, score) in &scored {
        if selected.len() >= MAX_ITEMS {
            break;
        }
        if *score < MIN_ADDITIONAL_CORE_SCORE && !selected.is_empty() {
            break;
        }
        let count = type_counts
            .get(memory.memory_type.as_str())
            .copied()
            .unwrap_or_default();
        if count >= MAX_PRIMARY_ITEMS_PER_TYPE {
            continue;
        }
        if push_selected_memory(&mut selected, &mut total_chars, memory) {
            selected_ids.insert(memory.id);
            *type_counts.entry(memory.memory_type.as_str()).or_default() += 1;
        }
    }

    for (memory, score) in &scored {
        if selected.len() >= MAX_ITEMS {
            break;
        }
        if *score < MIN_ADDITIONAL_CORE_SCORE && !selected.is_empty() {
            break;
        }
        if selected_ids.contains(&memory.id) {
            continue;
        }
        push_selected_memory(&mut selected, &mut total_chars, memory);
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

fn push_selected_memory<'a>(
    selected: &mut Vec<(&'a Memory, String)>,
    total_chars: &mut usize,
    memory: &'a Memory,
) -> bool {
    let preview: String = memory.text.chars().take(PREVIEW_LEN).collect();
    let item_len = preview.len() + memory.title.len() + 20;
    if *total_chars + item_len > MAX_CHARS && !selected.is_empty() {
        return false;
    }

    selected.push((memory, preview));
    *total_chars += item_len;
    true
}

fn calculate_memory_score(memory: &Memory, now_epoch: i64) -> f64 {
    let type_weight = match memory.memory_type.as_str() {
        "bugfix" => 3.0,
        "architecture" => 2.6,
        "decision" => 2.2,
        "discovery" => 1.8,
        "preference" => 1.4,
        _ => 0.5,
    };

    let age_days = (now_epoch - memory.updated_at_epoch) / 86400;
    let time_decay = if age_days <= 7 {
        1.0
    } else if age_days <= 30 {
        0.55
    } else {
        0.4
    };

    let meta_penalty = if is_memory_self_diagnostic(memory) {
        0.35
    } else {
        1.0
    };

    type_weight * time_decay * meta_penalty
}
