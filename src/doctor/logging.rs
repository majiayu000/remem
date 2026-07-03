use super::types::{Check, Status};

pub(super) fn check_log_health() -> Check {
    let Some(snapshot) = crate::log::log_health_snapshot() else {
        return Check::new(
            "Log health",
            Status::Warn,
            "log path unavailable; REMEM_DATA_DIR could not be resolved",
        );
    };

    let mut status = Status::Ok;
    let mut details = vec![format!(
        "path: {}; active: {} bytes; retained+active: {} bytes; max active: {} bytes; rotated files: {}; lock timeout: {}ms",
        snapshot.path.display(),
        snapshot.active_bytes,
        snapshot.total_bytes,
        snapshot.max_bytes,
        snapshot.max_rotated_files,
        snapshot.lock_timeout_ms
    )];

    if !snapshot.invalid_env.is_empty() {
        status = Status::Warn;
        details.push(format!(
            "invalid env fallback: {}",
            snapshot
                .invalid_env
                .iter()
                .map(|item| format!("{} ({}; default {})", item.name, item.reason, item.default))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if let Some(error) = snapshot.issue_read_error.as_ref() {
        status = Status::Warn;
        details.push(format!("rotation issue unreadable: {error}"));
    }

    if let Some(issue) = snapshot.issue.as_ref() {
        if snapshot.issue_is_fresh {
            status = Status::Warn;
            details.push(format!(
                "recent rotation issue: {} at {} for {}: {}",
                issue.kind, issue.at_epoch, issue.path, issue.message
            ));
        } else {
            details.push(format!(
                "last rotation issue is stale: {} at {}",
                issue.kind, issue.at_epoch
            ));
        }
    }

    Check::new("Log health", status, details.join("; "))
}
