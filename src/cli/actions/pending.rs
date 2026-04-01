use anyhow::Result;

use crate::{db, pending_admin};

use crate::cli::types::PendingAction;

pub(in crate::cli) fn run_pending(action: PendingAction) -> Result<()> {
    let conn = db::open_db()?;

    match action {
        PendingAction::ListFailed { project, limit } => {
            let rows = pending_admin::list_failed(&conn, project.as_deref(), limit)?;
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
        PendingAction::RetryFailed { project, limit } => {
            let count = pending_admin::retry_failed(&conn, project.as_deref(), limit)?;
            println!("Moved {} failed rows back to pending.", count);
        }
        PendingAction::PurgeFailed {
            project,
            older_than_days,
        } => {
            let count = pending_admin::purge_failed(&conn, project.as_deref(), older_than_days)?;
            println!(
                "Purged {} failed rows older than {} day(s).",
                count, older_than_days
            );
        }
    }

    Ok(())
}
