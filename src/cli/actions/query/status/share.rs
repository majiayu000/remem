//! Compact, screenshot-friendly status card for `remem status --share`.
//! Deliberately omits database paths and project names so the output is
//! safe to post publicly.

use super::types::StatusReport;

const CARD_MIN_INNER_WIDTH: usize = 44;

pub(super) fn render_share_card(report: &StatusReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "remem v{} — memory for coding agents",
        report.version
    ));
    lines.push(String::new());
    lines.push(stat_line("Memories", report.totals.memories));
    lines.push(stat_line("Sessions", report.totals.sessions));
    lines.push(stat_line("Observations", report.totals.observations));
    lines.push(stat_line("Raw messages", report.totals.raw_messages));
    if report.today.new_memories > 0 {
        lines.push(stat_line_signed("Today", report.today.new_memories));
    }
    lines.push(String::new());
    lines.push("github.com/majiayu000/remem".to_string());

    let width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        .max(CARD_MIN_INNER_WIDTH);

    let mut card = String::new();
    card.push_str(&format!("╭{}╮\n", "─".repeat(width + 2)));
    for line in lines {
        card.push_str(&format!("│ {:<width$} │\n", line, width = width));
    }
    card.push_str(&format!("╰{}╯\n", "─".repeat(width + 2)));
    card
}

fn stat_line(label: &str, value: i64) -> String {
    format!("{:<14}{:>10}", label, format_count(value))
}

fn stat_line_signed(label: &str, value: i64) -> String {
    format!("{:<14}{:>10}", label, format!("+{}", format_count(value)))
}

pub(super) fn format_count(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    if negative {
        format!("-{grouped}")
    } else {
        grouped
    }
}
