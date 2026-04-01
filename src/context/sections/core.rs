use crate::memory::Memory;

use super::super::format::format_epoch_short;

const MAX_CHARS: usize = 3000;
const MAX_ITEMS: usize = 6;
const PREVIEW_LEN: usize = 200;

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

    let mut selected = Vec::new();
    let mut total_chars = 0;
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
