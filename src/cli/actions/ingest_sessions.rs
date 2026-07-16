//! CLI action for `remem ingest-sessions` (issue #722).

use anyhow::Result;

use crate::db;
use crate::ingest::sessions::{
    default_scan_roots, run_ingest_sessions, IngestOptions, IngestSummary, ScanRoot,
};
use crate::memory::raw_query::parse_time_lower_bound;

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
        since_epoch: since.map(parse_time_lower_bound).transpose()?,
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
