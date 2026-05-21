use super::super::format::{
    char_len, format_epoch_short, format_epoch_time, truncate_chars_with_ellipsis,
};
use super::super::types::SessionSummaryBrief;

const REQUEST_PREVIEW_CHARS: usize = 160;
const COMPLETED_PREVIEW_CHARS: usize = 120;

#[cfg(test)]
pub(in crate::context) fn render_recent_sessions(
    output: &mut String,
    summaries: &[SessionSummaryBrief],
) {
    render_recent_sessions_with_limit(output, summaries, usize::MAX);
}

pub(in crate::context) fn render_recent_sessions_with_limit(
    output: &mut String,
    summaries: &[SessionSummaryBrief],
    char_limit: usize,
) -> usize {
    if char_limit == 0 {
        return 0;
    }
    let header = "## Sessions\n";
    let header_chars = char_len(header);
    let trailer_chars = 1;
    if header_chars + trailer_chars >= char_limit {
        return 0;
    }

    let mut section = String::from(header);
    let mut total_chars = header_chars + trailer_chars;
    let mut rendered = 0usize;
    for summary in summaries {
        let date = format_epoch_short(summary.created_at_epoch);
        let time = format_epoch_time(summary.created_at_epoch);
        let completed_part = summary
            .completed
            .as_deref()
            .and_then(|completed| completed.lines().find(|line| !line.trim().is_empty()))
            .map(format_completed_line)
            .unwrap_or_default();
        let request = truncate_chars_with_ellipsis(&summary.request, REQUEST_PREVIEW_CHARS);
        let line = format!("- **{}** {} {}{}\n", date, time, request, completed_part);
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

fn format_completed_line(line: &str) -> String {
    let truncated = truncate_chars_with_ellipsis(line, COMPLETED_PREVIEW_CHARS);
    format!(" => {}", truncated)
}
