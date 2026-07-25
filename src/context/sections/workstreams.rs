use crate::workstream::WorkStream;

use super::super::format::{char_len, inline_context_text};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::context) struct WorkstreamRenderSummary {
    pub count: usize,
    pub ids: Vec<i64>,
    pub item_end_chars: Vec<usize>,
}

#[cfg(test)]
pub(in crate::context) fn render_workstreams(output: &mut String, workstreams: &[WorkStream]) {
    render_workstreams_with_limits(output, workstreams, usize::MAX, usize::MAX);
}

pub(in crate::context) fn render_workstreams_with_limits(
    output: &mut String,
    workstreams: &[WorkStream],
    item_limit: usize,
    char_limit: usize,
) -> usize {
    render_workstreams_with_summary(output, workstreams, item_limit, char_limit).count
}

pub(in crate::context) fn render_workstreams_with_summary(
    output: &mut String,
    workstreams: &[WorkStream],
    item_limit: usize,
    char_limit: usize,
) -> WorkstreamRenderSummary {
    if item_limit == 0 || char_limit == 0 {
        return WorkstreamRenderSummary::default();
    }

    let header = "## WorkStreams\n";
    let header_chars = char_len(header);
    let trailer_chars = 1;
    if header_chars + trailer_chars >= char_limit {
        return WorkstreamRenderSummary::default();
    }

    let output_start_chars = char_len(output);
    let mut section = String::from(header);
    let mut total_chars = header_chars + trailer_chars;
    let mut rendered = 0usize;
    let mut ids = Vec::new();
    let mut item_end_chars = Vec::new();

    for workstream in workstreams.iter().take(item_limit) {
        let line = format_workstream_line(workstream);
        let line_chars = char_len(&line);
        if total_chars + line_chars > char_limit {
            break;
        }
        section.push_str(&line);
        total_chars += line_chars;
        ids.push(workstream.id);
        item_end_chars.push(output_start_chars + total_chars - trailer_chars);
        rendered += 1;
    }

    if rendered == 0 {
        return WorkstreamRenderSummary::default();
    }

    section.push('\n');
    output.push_str(&section);
    WorkstreamRenderSummary {
        count: rendered,
        ids,
        item_end_chars,
    }
}

fn format_workstream_line(workstream: &WorkStream) -> String {
    let next = workstream
        .next_action
        .as_deref()
        .map(inline_context_text)
        .unwrap_or_default();
    let next_part = if next.is_empty() {
        String::new()
    } else {
        format!(" -> {}", next)
    };
    let blockers = workstream
        .blockers
        .as_deref()
        .map(inline_context_text)
        .unwrap_or_default();
    let blockers_part = if blockers.is_empty() {
        String::new()
    } else {
        format!(" | blockers: {}", blockers)
    };
    let title = inline_context_text(&workstream.title);
    format!(
        "- #{} [{}] {}{}{}\n",
        workstream.id,
        workstream.status.as_str(),
        title,
        next_part,
        blockers_part
    )
}
