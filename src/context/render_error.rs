//! User-visible SessionStart load-error rendering.

use super::format::truncate_chars_with_ellipsis;
use super::render::helpers::{build_context_header_with_style, context_source_note};
use super::types::{ContextLoadError, ContextRequest};

pub(super) fn context_error_output(
    request: &ContextRequest,
    errors: &[ContextLoadError],
) -> String {
    let mut output = String::new();
    output.push_str(&build_context_header_with_style(
        &request.project,
        request.current_branch.as_deref(),
        request.hook_source.as_deref(),
        request.host,
        request.use_colors,
    ));
    if let Some(note) = context_source_note(request.hook_source.as_deref()) {
        output.push_str(note);
        output.push('\n');
    }
    output.push('\n');
    render_context_load_errors(&mut output, errors);
    output
}

pub(super) fn render_context_load_errors(output: &mut String, errors: &[ContextLoadError]) {
    if errors.is_empty() {
        return;
    }

    output.push_str("## Context Load Errors\n");
    for error in errors {
        output.push_str("- ");
        output.push_str(error.section);
        output.push_str(": ");
        output.push_str(&truncate_chars_with_ellipsis(
            &error.message.replace('\n', " "),
            240,
        ));
        output.push('\n');
    }
    output.push('\n');
}
