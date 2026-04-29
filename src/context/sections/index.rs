use std::collections::HashMap;

use crate::memory::Memory;

use super::super::format::{format_epoch_short, type_label};
use super::super::policy::ContextLimits;

#[cfg(test)]
pub(in crate::context) fn render_memory_index(output: &mut String, memories: &[Memory]) -> usize {
    render_memory_index_with_limits(output, memories, &ContextLimits::default())
}

pub(in crate::context) fn render_memory_index_with_limits(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
) -> usize {
    if limits.memory_index_limit == 0 || limits.memory_index_char_limit == 0 {
        return 0;
    }

    let mut by_type: HashMap<&str, Vec<&Memory>> = HashMap::new();
    for memory in memories
        .iter()
        .filter(|memory| memory.memory_type != "preference")
        .take(limits.memory_index_limit)
    {
        by_type
            .entry(memory.memory_type.as_str())
            .or_default()
            .push(memory);
    }
    if by_type.is_empty() {
        return 0;
    }

    let display_order = [
        "decision",
        "bugfix",
        "architecture",
        "discovery",
        "session_activity",
    ];

    let mut body = String::new();
    let mut total_chars = 0usize;
    let mut rendered_count = 0usize;
    for memory_type in &display_order {
        if let Some(memories_for_type) = by_type.get(memory_type) {
            if total_chars >= limits.memory_index_char_limit {
                break;
            }
            rendered_count += push_memory_index_line(
                &mut body,
                type_label(memory_type),
                memory_type,
                memories_for_type,
                limits.memory_index_char_limit,
                &mut total_chars,
            );
        }
    }

    for (memory_type, memories_for_type) in &by_type {
        if !display_order.contains(memory_type) && total_chars < limits.memory_index_char_limit {
            rendered_count += push_memory_index_line(
                &mut body,
                memory_type,
                memory_type,
                memories_for_type,
                limits.memory_index_char_limit,
                &mut total_chars,
            );
        }
    }
    if rendered_count == 0 {
        return 0;
    }
    output.push_str("## Index\n");
    output.push_str(&body);
    output.push('\n');
    rendered_count
}

fn push_memory_index_line(
    output: &mut String,
    label: &str,
    memory_type: &str,
    memories: &[&Memory],
    max_chars: usize,
    total_chars: &mut usize,
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
        let item = format!("#{} {} ({})", memory.id, memory.title, date);
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
            let truncated = truncate_to_chars(&item, remaining);
            if truncated.is_empty() {
                break;
            }
            line.push_str(separator);
            line.push_str(&truncated);
            line_chars += char_len(separator) + char_len(&truncated);
            rendered += 1;
            break;
        }
        line.push_str(separator);
        line.push_str(&item);
        line_chars += next_len;
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

fn char_len(value: &str) -> usize {
    value.chars().count()
}

fn truncate_to_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    let mut truncated: String = value.chars().take(max_chars - 3).collect();
    truncated.push_str("...");
    truncated
}
