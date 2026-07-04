//! CLI action for `remem ingest-sessions` (issue #722).

use anyhow::{bail, Result};

use crate::db;
use crate::ingest::sessions::{
    default_scan_roots, run_ingest_sessions, IngestOptions, IngestSummary, ScanRoot,
};

/// Run one batch ingestion pass and return the summary so the dispatcher can
/// map partial failures to a non-zero exit code.
pub(in crate::cli) fn run_ingest_sessions_cli(
    roots: &[String],
    since: Option<&str>,
    json: bool,
) -> Result<IngestSummary> {
    let mut scan_roots = default_scan_roots();
    for spec in roots {
        scan_roots.push(ScanRoot::parse(spec)?);
    }
    let options = IngestOptions {
        since_epoch: since.map(parse_time_bound).transpose()?,
    };

    let conn = db::open_db()?;
    let summary = run_ingest_sessions(&conn, &scan_roots, &options)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "Scanned {} files: {} skipped (cursor/--since), {} new messages, {} failed files, {} partial (active tail).",
            summary.scanned,
            summary.skipped,
            summary.ingested_messages,
            summary.failed_files,
            summary.partial_files
        );
        if summary.failed_files > 0 {
            println!("Failed files are recorded in raw_ingest_failures and retried next run.");
        }
    }
    Ok(summary)
}

/// Parse a time bound given as Unix epoch seconds, an ISO8601 datetime, or a
/// plain `YYYY-MM-DD` date (interpreted as UTC midnight).
pub(in crate::cli) fn parse_time_bound(value: &str) -> Result<i64> {
    let trimmed = value.trim();
    if let Ok(epoch) = trimmed.parse::<i64>() {
        return Ok(epoch);
    }
    if let Ok(datetime) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(datetime.timestamp());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let midnight = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time of day");
        return Ok(midnight.and_utc().timestamp());
    }
    bail!("invalid time bound {trimmed:?}: expected Unix epoch, ISO8601 datetime, or YYYY-MM-DD");
}

#[cfg(test)]
mod tests {
    use super::parse_time_bound;

    #[test]
    fn parses_epoch_iso_and_date_bounds() {
        assert_eq!(parse_time_bound("1750000000").unwrap(), 1_750_000_000);
        assert_eq!(
            parse_time_bound("2026-01-02T03:04:05Z").unwrap(),
            1_767_323_045
        );
        assert_eq!(parse_time_bound("2026-01-02").unwrap(), 1_767_312_000);
        assert!(parse_time_bound("not-a-time").is_err());
    }
}
