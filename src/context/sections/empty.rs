use super::super::format::format_header_datetime;

pub(in crate::context) fn empty_state_output(project: &str) -> String {
    format!(
        "# [{}] context {}\nNo previous sessions found.\n",
        project,
        format_header_datetime()
    )
}
