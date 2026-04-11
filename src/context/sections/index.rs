use std::collections::HashMap;

use crate::memory::Memory;

use super::super::format::{format_epoch_short, type_label};

pub(in crate::context) fn render_memory_index(output: &mut String, memories: &[Memory]) {
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
