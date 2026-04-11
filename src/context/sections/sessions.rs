use super::super::format::{format_epoch_short, format_epoch_time};
use super::super::types::SessionSummaryBrief;

pub(in crate::context) fn render_recent_sessions(
    output: &mut String,
    summaries: &[SessionSummaryBrief],
) {
    output.push_str("## Sessions\n");
    for summary in summaries {
        let date = format_epoch_short(summary.created_at_epoch);
        let time = format_epoch_time(summary.created_at_epoch);
        let completed_part = summary
            .completed
            .as_deref()
            .and_then(|completed| completed.lines().find(|line| !line.trim().is_empty()))
            .map(format_completed_line)
            .unwrap_or_default();
        output.push_str(&format!(
            "- **{}** {} {}{}\n",
            date, time, summary.request, completed_part
        ));
    }
    output.push('\n');
}

fn format_completed_line(line: &str) -> String {
    let truncated: String = line.chars().take(120).collect();
    if line.len() > 120 {
        format!(" => {}...", truncated)
    } else {
        format!(" => {}", truncated)
    }
}
