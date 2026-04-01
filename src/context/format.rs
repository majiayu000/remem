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
