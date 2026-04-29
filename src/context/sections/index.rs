use std::collections::HashMap;

use crate::memory::Memory;

use super::super::format::{format_epoch_short, type_label};
use super::super::policy::ContextLimits;

#[cfg(test)]
pub(in crate::context) fn render_memory_index(output: &mut String, memories: &[Memory]) {
    render_memory_index_with_limits(output, memories, &ContextLimits::default());
}

pub(in crate::context) fn render_memory_index_with_limits(
    output: &mut String,
    memories: &[Memory],
    limits: &ContextLimits,
) {
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
        return;
    }

    let display_order = [
        "decision",
        "bugfix",
        "architecture",
        "discovery",
        "session_activity",
    ];

    output.push_str("## Index\n");
    let mut total_chars = 0usize;
    for memory_type in &display_order {
        if let Some(memories_for_type) = by_type.get(memory_type) {
            if total_chars >= limits.memory_index_char_limit {
                break;
            }
            push_memory_index_line(
                output,
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
            push_memory_index_line(
                output,
                memory_type,
                memory_type,
                memories_for_type,
                limits.memory_index_char_limit,
                &mut total_chars,
            );
        }
    }
    output.push('\n');
}

fn push_memory_index_line(
    output: &mut String,
    label: &str,
    memory_type: &str,
    memories: &[&Memory],
    max_chars: usize,
    total_chars: &mut usize,
) {
    let section_label = if label == memory_type {
        memory_type.to_string()
    } else {
        label.to_string()
    };
    let prefix = format!("**{}** ({}): ", section_label, memories.len());
    if *total_chars + prefix.len() > max_chars && *total_chars > 0 {
        return;
    }
    output.push_str(&prefix);
    *total_chars += prefix.len();

    let mut first = true;
    for memory in memories.iter().take(10) {
        let date = format_epoch_short(memory.updated_at_epoch);
        let item = format!("#{} {} ({})", memory.id, memory.title, date);
        let separator = if first { "" } else { " | " };
        let next_len = separator.len() + item.len();
        if *total_chars + next_len > max_chars && !first {
            break;
        }
        output.push_str(separator);
        output.push_str(&item);
        *total_chars += next_len;
        first = false;
    }
    output.push('\n');
    *total_chars += 1;
}
