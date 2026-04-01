use crate::workstream::WorkStream;

pub(in crate::context) fn render_workstreams(output: &mut String, workstreams: &[WorkStream]) {
    output.push_str("## WorkStreams\n");
    for workstream in workstreams {
        let next = workstream.next_action.as_deref().unwrap_or("");
        let next_part = if next.is_empty() {
            String::new()
        } else {
            format!(" -> {}", next)
        };
        output.push_str(&format!(
            "- #{} [{}] {}{}\n",
            workstream.id,
            workstream.status.as_str(),
            workstream.title,
            next_part
        ));
    }
    output.push('\n');
}
