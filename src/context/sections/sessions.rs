use super::super::format::{format_epoch_short, format_epoch_time};
use super::super::types::SessionSummaryBrief;

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
) {
    if char_limit == 0 {
        return;
    }
    let header = "## Sessions\n";
    let header_chars = header.chars().count();
    let trailer_chars = 1;
    if header_chars + trailer_chars >= char_limit {
        return;
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
        let line = format!(
            "- **{}** {} {}{}\n",
            date, time, summary.request, completed_part
        );
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

fn format_completed_line(line: &str) -> String {
    let truncated: String = line.chars().take(120).collect();
    if line.len() > 120 {
        format!(" => {}...", truncated)
    } else {
        format!(" => {}", truncated)
    }
}
