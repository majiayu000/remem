use super::super::format::format_header_datetime;

pub(in crate::context) fn render_empty_state(project: &str) {
    println!(
        "# [{}] context {}\nNo previous sessions found.",
        project,
        format_header_datetime()
    );
}
