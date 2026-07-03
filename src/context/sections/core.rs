use crate::memory::{Memory, MemoryStalenessLabel, MemoryType};
use std::collections::HashMap;

use super::super::audit::memory_render_metadata_with_labels;
use super::super::format::{char_len, format_epoch_short, truncate_chars_with_ellipsis};
use super::super::memory_traits::is_memory_self_diagnostic;
use super::super::policy::ContextLimits;

const PREVIEW_LEN: usize = 200;
const MAX_PRIMARY_ITEMS_PER_TYPE: usize = 2;
const MIN_ADDITIONAL_CORE_SCORE: f64 = 1.3;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::context) struct CoreRenderSummary {
    pub count: usize,
    pub ids: Vec<i64>,
}

#[cfg(test)]
pub(in crate::context) fn render_core_memory(output: &mut String, memories: &[Memory]) {
    render_core_memory_with_limits(output, memories, &ContextLimits::default());
}

#[cfg(test)]
pub(in crate::context) fn render_core_memory_with_limits(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
) -> CoreRenderSummary {
    render_core_memory_with_limits_and_staleness(
        output,
        memories,
        limits,
        chrono::Utc::now().timestamp(),
        &HashMap::new(),
    )
}

pub(in crate::context) fn render_core_memory_with_limits_and_staleness(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
    render_reference_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> CoreRenderSummary {
    if limits.core_item_limit == 0 || limits.core_char_limit == 0 {
        return CoreRenderSummary::default();
    }
    let header = "## Core\n";
    let trailer_chars = 1;
    let header_chars = char_len(header);
    if header_chars + trailer_chars >= limits.core_char_limit {
        return CoreRenderSummary::default();
    }

    let mut scored: Vec<(usize, &Memory, i64, f64)> = memories
        .iter()
        .enumerate()
        .filter_map(|(retrieval_rank, memory)| {
            let memory_type = MemoryType::parse(&memory.memory_type)?;
            if !memory_type.is_core() {
                return None;
            }
            let score = calculate_memory_score(memory, memory_type, render_reference_epoch);
            Some((retrieval_rank, memory, score_bucket(score), score))
        })
        .collect();
    scored.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(&right.0)));

    let mut selected: Vec<(&Memory, String)> = Vec::new();
    let mut total_chars = header_chars + trailer_chars;
    let mut selected_ids = std::collections::HashSet::new();
    let mut type_counts: HashMap<&str, usize> = HashMap::new();

    for (_retrieval_rank, memory, _score_bucket, score) in &scored {
        if selected.len() >= limits.core_item_limit {
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
        if push_selected_memory(
            &mut selected,
            &mut total_chars,
            memory,
            limits.core_char_limit,
            render_reference_epoch,
            staleness_labels,
        ) {
            selected_ids.insert(memory.id);
            *type_counts.entry(memory.memory_type.as_str()).or_default() += 1;
        }
    }

    for (_retrieval_rank, memory, _score_bucket, score) in &scored {
        if selected.len() >= limits.core_item_limit {
            break;
        }
        if *score < MIN_ADDITIONAL_CORE_SCORE && !selected.is_empty() {
            break;
        }
        if selected_ids.contains(&memory.id) {
            continue;
        }
        push_selected_memory(
            &mut selected,
            &mut total_chars,
            memory,
            limits.core_char_limit,
            render_reference_epoch,
            staleness_labels,
        );
    }

    if selected.is_empty() {
        return CoreRenderSummary::default();
    }

    output.push_str(header);
    let selected_count = selected.len();
    let selected_ids = selected
        .iter()
        .map(|(memory, _)| memory.id)
        .collect::<Vec<_>>();
    for (memory, preview) in selected {
        let date = format_epoch_short(memory.updated_at_epoch);
        output.push_str(&format!(
            "**#{} {}** ({}, {}; {})\n",
            memory.id,
            memory.title,
            memory.memory_type,
            date,
            memory_render_metadata_with_labels(memory, render_reference_epoch, staleness_labels)
        ));
        output.push_str(&preview);
        output.push('\n');
    }
    output.push('\n');
    CoreRenderSummary {
        count: selected_count,
        ids: selected_ids,
    }
}

fn push_selected_memory<'a>(
    selected: &mut Vec<(&'a Memory, String)>,
    total_chars: &mut usize,
    memory: &'a Memory,
    max_chars: usize,
    now_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> bool {
    let header = format!(
        "**#{} {}** ({}, {}; {})\n",
        memory.id,
        memory.title,
        memory.memory_type,
        format_epoch_short(memory.updated_at_epoch),
        memory_render_metadata_with_labels(memory, now_epoch, staleness_labels)
    );
    let fixed_chars = char_len(&header) + 1;
    if *total_chars + fixed_chars >= max_chars {
        return false;
    }

    let remaining_chars = max_chars - *total_chars - fixed_chars;
    let preview_limit = remaining_chars.min(PREVIEW_LEN);
    let preview = truncate_chars_with_ellipsis(&memory.text, preview_limit);
    if preview.is_empty() {
        return false;
    }
    let item_len = char_len(&preview) + fixed_chars;
    selected.push((memory, preview));
    *total_chars += item_len;
    true
}

fn calculate_memory_score(memory: &Memory, memory_type: MemoryType, now_epoch: i64) -> f64 {
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

    memory_type.weight() * time_decay * meta_penalty
}

fn score_bucket(score: f64) -> i64 {
    (score * 100.0).round() as i64
}
