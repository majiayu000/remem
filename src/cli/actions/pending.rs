use anyhow::Result;
use serde::Serialize;

use crate::cli::types::PendingAction;
use crate::db::{
    self,
    pending::admin::{FailedPendingRow, LegacyPendingMigration},
    ExtractionReplayRange, ExtractionReplayRangeEvidence,
};

const LIST_EXTRACTION_RANGES_DEFAULT_LIMIT: i64 = 20;
const MUTATE_EXTRACTION_RANGES_DEFAULT_LIMIT: i64 = 100;

pub(in crate::cli) fn run_pending(action: PendingAction) -> Result<()> {
    match action {
        PendingAction::ListFailed {
            project,
            limit,
            json,
        } => {
            let conn = db::open_db_read_only()?;
            let rows = db::pending::admin::list_failed(&conn, project.as_deref(), limit)?;
            if json {
                let output = PendingListFailedJson {
                    project,
                    limit: limit.max(1),
                    count: rows.len(),
                    failed: rows,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
                return Ok(());
            }
            if rows.is_empty() {
                println!("No failed pending observations.");
                return Ok(());
            }
            println!("Failed pending observations ({}):", rows.len());
            for row in rows {
                let ts = chrono::DateTime::from_timestamp(row.updated_at_epoch, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let err = row
                    .last_error
                    .as_deref()
                    .map(|message| db::truncate_str(message, 120).to_string())
                    .unwrap_or_default();
                println!(
                    "  [{}] {} | {} | {} | attempt={} | {}",
                    row.id, row.project, row.session_id, row.tool_name, row.attempt_count, ts
                );
                if !err.is_empty() {
                    println!("      error: {}", err);
                }
            }
        }
        PendingAction::RetryFailed {
            project,
            limit,
            dry_run,
        } => {
            if dry_run {
                let conn = db::open_db_read_only()?;
                let count = db::pending::admin::count_failed_retry_candidates(
                    &conn,
                    project.as_deref(),
                    limit,
                )?;
                println!(
                    "Would move {} failed row(s) back to pending for legacy migration.",
                    count
                );
                println!(
                    "Next after applying retry-failed: run `remem pending migrate-legacy --dry-run`."
                );
            } else {
                let conn = db::open_db()?;
                let count = db::pending::admin::retry_failed(&conn, project.as_deref(), limit)?;
                println!(
                    "Moved {} failed row(s) back to pending for legacy migration.",
                    count
                );
                if count > 0 {
                    println!(
                        "Next: run `remem pending migrate-legacy --dry-run`, then `remem pending migrate-legacy` to replay them into captured_events."
                    );
                }
            }
        }
        PendingAction::PurgeFailed {
            project,
            older_than_days,
            dry_run,
        } => {
            if dry_run {
                let conn = db::open_db_read_only()?;
                let count = db::pending::admin::count_failed_purge_candidates(
                    &conn,
                    project.as_deref(),
                    older_than_days,
                )?;
                println!(
                    "Would purge {} failed rows older than {} day(s).",
                    count, older_than_days
                );
            } else {
                let conn = db::open_db()?;
                let count =
                    db::pending::admin::purge_failed(&conn, project.as_deref(), older_than_days)?;
                println!(
                    "Purged {} failed rows older than {} day(s).",
                    count, older_than_days
                );
            }
        }
        PendingAction::MigrateLegacy {
            project,
            host,
            limit,
            dry_run,
            json,
        } => {
            if dry_run {
                let conn = db::open_db_read_only()?;
                let count = db::pending::admin::count_legacy_migration_candidates(
                    &conn,
                    project.as_deref(),
                    limit,
                )?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&PendingMigrateLegacyJson {
                            project,
                            limit: limit.max(1),
                            count,
                            migrated: Vec::new(),
                        })?
                    );
                } else {
                    println!("Would migrate {} legacy pending row(s).", count);
                }
            } else {
                let mut conn = db::open_db()?;
                let migrated = db::pending::admin::migrate_legacy_pending(
                    &mut conn,
                    project.as_deref(),
                    host.as_deref(),
                    limit,
                )?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&PendingMigrateLegacyJson {
                            project,
                            limit: limit.max(1),
                            count: migrated.len(),
                            migrated,
                        })?
                    );
                } else {
                    println!(
                        "Migrated {} legacy pending row(s) into captured_events.",
                        migrated.len()
                    );
                }
            }
        }
        PendingAction::ListExtractionRanges {
            id,
            project,
            limit,
            json,
        } => {
            let conn = db::open_db_read_only()?;
            if let Some(range_id) = id {
                let evidence = db::get_extraction_replay_range_evidence(&conn, range_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&evidence)?);
                } else {
                    print_exact_extraction_range(&evidence);
                }
                return Ok(());
            }
            let limit = limit.unwrap_or(LIST_EXTRACTION_RANGES_DEFAULT_LIMIT);
            let ranges = db::list_extraction_replay_ranges(&conn, project.as_deref(), limit)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&PendingExtractionRangesJson {
                        project,
                        limit: limit.max(1),
                        count: ranges.len(),
                        ranges,
                    })?
                );
                return Ok(());
            }
            if ranges.is_empty() {
                println!("No exhausted extraction ranges.");
                return Ok(());
            }
            println!("Exhausted extraction ranges ({}):", ranges.len());
            for range in ranges {
                let err = range
                    .last_error
                    .as_deref()
                    .map(|message| db::truncate_str(message, 120).to_string())
                    .unwrap_or_default();
                println!(
                    "  [{}] {} | {} | {} | events={}..{} | status={} | attempts={}",
                    range.id,
                    range.project,
                    range.session_id.as_deref().unwrap_or("<none>"),
                    range.task_kind,
                    range.from_event_id,
                    range.to_event_id,
                    range.status,
                    range.attempts
                );
                if !err.is_empty() {
                    println!("      error: {}", err);
                }
            }
        }
        PendingAction::RetryExtractionRanges {
            id,
            project,
            limit,
            acknowledge_quarantine,
            dry_run,
        } => {
            let limit = limit.unwrap_or(MUTATE_EXTRACTION_RANGES_DEFAULT_LIMIT);
            if dry_run {
                let conn = db::open_db_read_only()?;
                if let Some(range_id) = id {
                    db::ensure_extraction_replay_range_retryable(
                        &conn,
                        range_id,
                        acknowledge_quarantine,
                    )?;
                    println!("Would requeue exhausted extraction range {range_id}.");
                } else {
                    let count = db::count_retryable_extraction_replay_ranges(
                        &conn,
                        project.as_deref(),
                        limit,
                    )?;
                    println!("Would requeue {} exhausted extraction range(s).", count);
                }
            } else {
                let conn = db::open_db()?;
                if let Some(range_id) = id {
                    db::retry_extraction_replay_range(&conn, range_id, acknowledge_quarantine)?;
                    println!("Requeued exhausted extraction range {range_id}.");
                } else {
                    let count =
                        db::retry_extraction_replay_ranges(&conn, project.as_deref(), limit)?;
                    println!("Requeued {} exhausted extraction range(s).", count);
                }
            }
        }
        PendingAction::QuarantineExtractionRanges {
            id,
            project,
            limit,
            dry_run,
        } => {
            let limit = limit.unwrap_or(MUTATE_EXTRACTION_RANGES_DEFAULT_LIMIT);
            if dry_run {
                let conn = db::open_db_read_only()?;
                if let Some(range_id) = id {
                    db::ensure_extraction_replay_range_retryable(&conn, range_id, false)?;
                    println!("Would quarantine exhausted extraction range {range_id}.");
                } else {
                    let count = db::count_retryable_extraction_replay_ranges(
                        &conn,
                        project.as_deref(),
                        limit,
                    )?;
                    println!("Would quarantine {} exhausted extraction range(s).", count);
                }
            } else {
                let conn = db::open_db()?;
                if let Some(range_id) = id {
                    db::quarantine_extraction_replay_range(&conn, range_id)?;
                    println!("Quarantined exhausted extraction range {range_id}.");
                } else {
                    let count =
                        db::quarantine_extraction_replay_ranges(&conn, project.as_deref(), limit)?;
                    println!("Quarantined {} exhausted extraction range(s).", count);
                }
            }
        }
    }

    Ok(())
}

fn print_exact_extraction_range(evidence: &ExtractionReplayRangeEvidence) {
    let range = &evidence.range;
    println!(
        "Extraction replay range [{}] {} | {} | {} | events={}..{} | status={} | attempts={}",
        range.id,
        range.project,
        range.session_id.as_deref().unwrap_or("<none>"),
        range.task_kind,
        range.from_event_id,
        range.to_event_id,
        range.status,
        range.attempts
    );
    if let Some(error) = range.last_error.as_deref() {
        println!("  range error: {}", db::truncate_str(error, 120));
    }
    if let Some(task) = &evidence.replay_task {
        println!(
            "  replay task [{}] status={} | attempts={}",
            task.id, task.status, task.attempts
        );
        if let Some(error) = task.last_error.as_deref() {
            println!("  task error: {}", db::truncate_str(error, 120));
        }
    } else {
        println!("  replay task: <none>");
    }
}

#[derive(Debug, Clone, Serialize)]
struct PendingListFailedJson {
    project: Option<String>,
    limit: i64,
    count: usize,
    failed: Vec<FailedPendingRow>,
}

#[derive(Debug, Clone, Serialize)]
struct PendingMigrateLegacyJson {
    project: Option<String>,
    limit: i64,
    count: usize,
    migrated: Vec<LegacyPendingMigration>,
}

#[derive(Debug, Clone, Serialize)]
struct PendingExtractionRangesJson {
    project: Option<String>,
    limit: i64,
    count: usize,
    ranges: Vec<ExtractionReplayRange>,
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn cli_pending_list_failed_json_is_machine_parseable(
    ) -> std::result::Result<(), serde_json::Error> {
        let output = PendingListFailedJson {
            project: Some("proj".to_string()),
            limit: 1,
            count: 1,
            failed: vec![FailedPendingRow {
                id: 1,
                session_id: "session-1".to_string(),
                project: "proj".to_string(),
                tool_name: "Bash".to_string(),
                attempt_count: 3,
                updated_at_epoch: 10,
                last_error: Some("failed".to_string()),
            }],
        };

        let text = serde_json::to_string(&output)?;
        let parsed: Value = serde_json::from_str(&text)?;

        assert_eq!(parsed["project"], "proj");
        assert_eq!(parsed["count"], 1);
        assert_eq!(parsed["failed"][0]["tool_name"], "Bash");
        Ok(())
    }

    #[test]
    fn cli_pending_migrate_legacy_json_is_machine_parseable(
    ) -> std::result::Result<(), serde_json::Error> {
        let output = PendingMigrateLegacyJson {
            project: Some("proj".to_string()),
            limit: 1,
            count: 1,
            migrated: vec![LegacyPendingMigration {
                pending_id: 7,
                event_id: "legacy-pending-7".to_string(),
                captured_event_id: 11,
                extraction_task_id: 13,
                host: "codex-cli".to_string(),
                project: "proj".to_string(),
                session_id: "session-1".to_string(),
            }],
        };

        let text = serde_json::to_string(&output)?;
        let parsed: Value = serde_json::from_str(&text)?;

        assert_eq!(parsed["count"], 1);
        assert_eq!(parsed["migrated"][0]["event_id"], "legacy-pending-7");
        assert_eq!(parsed["migrated"][0]["host"], "codex-cli");
        Ok(())
    }

    #[test]
    fn exact_extraction_range_json_has_terminal_task_evidence() {
        let evidence = ExtractionReplayRangeEvidence {
            range: ExtractionReplayRange {
                id: 308,
                source_task_id: 10,
                replay_task_id: Some(11),
                task_kind: "observation_extract".to_string(),
                project: "/repo".to_string(),
                session_id: Some("session".to_string()),
                from_event_id: 20,
                to_event_id: 30,
                status: "replayed".to_string(),
                attempts: 1,
                updated_at_epoch: 40,
                last_error: None,
            },
            replay_task: Some(db::ExtractionReplayTaskEvidence {
                id: 11,
                status: "done".to_string(),
                attempts: 0,
                last_error: None,
            }),
        };

        let value = serde_json::to_value(evidence).expect("serialize exact evidence");
        assert_eq!(value["range"]["id"], 308);
        assert_eq!(value["range"]["status"], "replayed");
        assert_eq!(value["replay_task"]["id"], 11);
        assert_eq!(value["replay_task"]["status"], "done");
        assert!(value.get("payload").is_none());
        assert!(value.get("provider").is_none());
    }
}
