use crate::workstream::WorkStream;

use super::super::format::char_len;

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
    if item_limit == 0 || char_limit == 0 {
        return 0;
    }

    let header = "## WorkStreams\n";
    let header_chars = char_len(header);
    let trailer_chars = 1;
    if header_chars + trailer_chars >= char_limit {
        return 0;
    }

    let mut section = String::from(header);
    let mut total_chars = header_chars + trailer_chars;
    let mut rendered = 0usize;

    for workstream in workstreams.iter().take(item_limit) {
        let line = format_workstream_line(workstream);
        let line_chars = char_len(&line);
        if total_chars + line_chars > char_limit {
            break;
        }
        section.push_str(&line);
        total_chars += line_chars;
        rendered += 1;
    }

    if rendered == 0 {
        return 0;
    }

    section.push('\n');
    output.push_str(&section);
    rendered
}

fn format_workstream_line(workstream: &WorkStream) -> String {
    let next = workstream.next_action.as_deref().unwrap_or("");
    let next_part = if next.is_empty() {
        String::new()
    } else {
        format!(" -> {}", next)
    };
    format!(
        "- #{} [{}] {}{}\n",
        workstream.id,
        workstream.status.as_str(),
        workstream.title,
        next_part
    )
}
