use anyhow::Error;

use super::constants::PENDING_RETRY_MAX_SECS;

pub(crate) fn is_ai_timeout_error(err: &Error) -> bool {
    err.to_string().to_lowercase().contains("timed out")
}

pub(crate) fn pending_retry_backoff_secs(attempt_count: i64) -> i64 {
    let secs = match attempt_count {
        i64::MIN..=1 => 30,
        2 => 120,
        3 => 300,
        4 => 900,
        _ => 1800,
    };
    secs.min(PENDING_RETRY_MAX_SECS)
}
