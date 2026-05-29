pub(in crate::context) fn empty_state_output(header: &str, source_note: Option<&str>) -> String {
    let mut output = String::new();
    output.push_str(header);
    if let Some(note) = source_note {
        output.push_str(note);
        output.push('\n');
    }
    output.push_str("No previous sessions found.\n");
    output
}
