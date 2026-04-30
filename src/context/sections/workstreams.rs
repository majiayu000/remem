use crate::workstream::WorkStream;

#[cfg(test)]
pub(in crate::context) fn render_workstreams(output: &mut String, workstreams: &[WorkStream]) {
    render_workstreams_with_limits(output, workstreams, usize::MAX, usize::MAX);
}

pub(in crate::context) fn render_workstreams_with_limits(
    output: &mut String,
    workstreams: &[WorkStream],
    item_limit: usize,
    char_limit: usize,
) {
    if item_limit == 0 || char_limit == 0 {
        return;
    }

    let header = "## WorkStreams\n";
    let header_chars = header.chars().count();
    let trailer_chars = 1;
    if header_chars + trailer_chars >= char_limit {
        return;
    }

    let mut section = String::from(header);
    let mut total_chars = header_chars + trailer_chars;
    let mut rendered = 0usize;

    for workstream in workstreams.iter().take(item_limit) {
        let line = format_workstream_line(workstream);
        let line_chars = line.chars().count();
        if total_chars + line_chars > char_limit {
            break;
        }
        section.push_str(&line);
        total_chars += line_chars;
        rendered += 1;
    }

    if rendered == 0 {
        return;
    }

    section.push('\n');
    output.push_str(&section);
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
