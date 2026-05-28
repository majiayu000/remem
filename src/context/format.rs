use chrono::{Local, TimeZone};

pub(super) fn format_header_datetime() -> String {
    Local::now().format("%Y-%m-%d %-I:%M%P %:z").to_string()
}

pub(super) fn type_label(memory_type: &str) -> &'static str {
    match memory_type {
        "decision" => "Decisions",
        "bugfix" => "Bug Fixes",
        "architecture" => "Architecture",
        "discovery" => "Discoveries",
        "lesson" => "Lessons",
        "preference" => "Preferences",
        "session_activity" => "Sessions",
        _ => "Other",
    }
}

pub(super) fn format_epoch_short(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|dt| dt.format("%m-%d").to_string())
        .unwrap_or_default()
}

pub(super) fn format_epoch_time(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|dt| dt.format("%-I:%M%P").to_string())
        .unwrap_or_default()
}

pub(super) fn char_len(value: &str) -> usize {
    value.chars().count()
}

pub(super) fn truncate_chars_with_ellipsis(value: &str, max_chars: usize) -> String {
    if char_len(value) <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    let mut truncated: String = value.chars().take(max_chars - 3).collect();
    truncated.push_str("...");
    truncated
}
