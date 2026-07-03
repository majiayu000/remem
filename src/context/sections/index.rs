use std::collections::{HashMap, HashSet};

use crate::memory::{Memory, MemoryStalenessLabel, MemoryType};

use super::super::audit::memory_render_metadata_with_labels;
use super::super::format::{
    char_len, format_epoch_short, truncate_chars_with_ellipsis, type_label,
};
use super::super::policy::ContextLimits;

#[cfg(test)]
pub(in crate::context) fn render_memory_index(output: &mut String, memories: &[Memory]) -> usize {
    render_memory_index_with_limits(output, memories, &ContextLimits::default())
}

#[cfg(test)]
pub(in crate::context) fn render_memory_index_with_limits(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
) -> usize {
    let excluded_ids = HashSet::new();
    render_memory_index_with_limits_excluding(output, memories, limits, &excluded_ids)
}

#[cfg(test)]
pub(in crate::context) fn render_memory_index_with_limits_excluding(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
    excluded_ids: &HashSet<i64>,
) -> usize {
    render_memory_index_with_summary(output, memories, limits, excluded_ids).count
}

pub(in crate::context) fn render_memory_index_with_limits_excluding_and_staleness(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
    excluded_ids: &HashSet<i64>,
    render_reference_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> usize {
    render_memory_index_with_summary_and_staleness(
        output,
        memories,
        limits,
        excluded_ids,
        render_reference_epoch,
        staleness_labels,
    )
    .count
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::context) struct IndexRenderSummary {
    pub count: usize,
    pub ids: Vec<i64>,
}

#[cfg(test)]
pub(in crate::context) fn render_memory_index_with_summary(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
    excluded_ids: &HashSet<i64>,
) -> IndexRenderSummary {
    render_memory_index_with_summary_and_staleness(
        output,
        memories,
        limits,
        excluded_ids,
        chrono::Utc::now().timestamp(),
        &HashMap::new(),
    )
}

pub(in crate::context) fn render_memory_index_with_summary_and_staleness(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
    excluded_ids: &HashSet<i64>,
    render_reference_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> IndexRenderSummary {
    if limits.memory_index_limit == 0 || limits.memory_index_char_limit == 0 {
        return IndexRenderSummary::default();
    }

    let mut indexed_memories = memories
        .iter()
        .filter(|memory| MemoryType::parse(&memory.memory_type).is_none_or(MemoryType::is_indexed))
        .filter(|memory| !excluded_ids.contains(&memory.id))
        .collect::<Vec<_>>();
    indexed_memories.sort_by(|left, right| {
        memory_index_type_order(left)
            .cmp(&memory_index_type_order(right))
            .then_with(|| left.memory_type.cmp(&right.memory_type))
            .then_with(|| right.updated_at_epoch.cmp(&left.updated_at_epoch))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut by_type: HashMap<&str, Vec<&Memory>> = HashMap::new();
    for memory in indexed_memories.into_iter().take(limits.memory_index_limit) {
        by_type
            .entry(memory.memory_type.as_str())
            .or_default()
            .push(memory);
    }
    if by_type.is_empty() {
        return IndexRenderSummary::default();
    }

    let mut display_order = MemoryType::ALL
        .iter()
        .copied()
        .filter(|memory_type| memory_type.is_indexed())
        .collect::<Vec<_>>();
    display_order.sort_by_key(|memory_type| memory_type.index_order().unwrap_or(usize::MAX));

    let mut body = String::new();
    let mut total_chars = 0usize;
    let mut rendered_count = 0usize;
    let mut rendered_ids = Vec::new();
    let mut ordered_types = HashSet::new();
    for memory_type in display_order {
        let memory_type_key = memory_type.as_str();
        ordered_types.insert(memory_type_key);
        if let Some(memories_for_type) = by_type.get(memory_type_key) {
            if total_chars >= limits.memory_index_char_limit {
                break;
            }
            rendered_count += push_memory_index_line(
                &mut body,
                type_label(memory_type_key),
                memory_type_key,
                memories_for_type,
                limits.memory_index_char_limit,
                &mut total_chars,
                &mut rendered_ids,
                render_reference_epoch,
                staleness_labels,
            );
        }
    }

    let mut unordered_types = by_type
        .keys()
        .copied()
        .filter(|memory_type| !ordered_types.contains(memory_type))
        .collect::<Vec<_>>();
    unordered_types.sort_unstable();
    for memory_type in unordered_types {
        if total_chars < limits.memory_index_char_limit {
            if let Some(memories_for_type) = by_type.get(memory_type) {
                rendered_count += push_memory_index_line(
                    &mut body,
                    memory_type,
                    memory_type,
                    memories_for_type,
                    limits.memory_index_char_limit,
                    &mut total_chars,
                    &mut rendered_ids,
                    render_reference_epoch,
                    staleness_labels,
                );
            }
        }
    }
    if rendered_count == 0 {
        return IndexRenderSummary::default();
    }
    output.push_str("## Index\n");
    output.push_str(&body);
    output.push('\n');
    IndexRenderSummary {
        count: rendered_count,
        ids: rendered_ids,
    }
}

fn memory_index_type_order(memory: &Memory) -> usize {
    MemoryType::parse(&memory.memory_type)
        .and_then(|memory_type| memory_type.index_order())
        .unwrap_or(usize::MAX)
}

fn push_memory_index_line(
    output: &mut String,
    label: &str,
    memory_type: &str,
    memories: &[&Memory],
    max_chars: usize,
    total_chars: &mut usize,
    rendered_ids: &mut Vec<i64>,
    now_epoch: i64,
    staleness_labels: &HashMap<i64, MemoryStalenessLabel>,
) -> usize {
    let section_label = if label == memory_type {
        memory_type.to_string()
    } else {
        label.to_string()
    };
    let prefix = format!("**{}** ({}): ", section_label, memories.len());
    let prefix_len = char_len(&prefix);
    if *total_chars + prefix_len >= max_chars {
        return 0;
    }
    let mut line = prefix;
    let mut line_chars = prefix_len;

    let mut rendered = 0usize;
    let mut first = true;
    for memory in memories.iter().take(10) {
        let date = format_epoch_short(memory.updated_at_epoch);
        let item = format!(
            "#{} {} ({}; {})",
            memory.id,
            memory.title,
            date,
            memory_render_metadata_with_labels(memory, now_epoch, staleness_labels)
        );
        let separator = if first { "" } else { " | " };
        let next_len = char_len(separator) + char_len(&item);
        if *total_chars + line_chars + next_len > max_chars {
            if !first {
                break;
            }
            let remaining =
                max_chars.saturating_sub(*total_chars + line_chars + char_len(separator));
            if remaining == 0 {
                break;
            }
            let truncated = truncate_chars_with_ellipsis(&item, remaining);
            if truncated.is_empty() {
                break;
            }
            line.push_str(separator);
            line.push_str(&truncated);
            line_chars += char_len(separator) + char_len(&truncated);
            rendered_ids.push(memory.id);
            rendered += 1;
            break;
        }
        line.push_str(separator);
        line.push_str(&item);
        line_chars += next_len;
        rendered_ids.push(memory.id);
        rendered += 1;
        first = false;
    }
    if rendered == 0 {
        return 0;
    }
    if *total_chars + line_chars < max_chars {
        line.push('\n');
        line_chars += 1;
    }
    output.push_str(&line);
    *total_chars += line_chars;
    rendered
}
