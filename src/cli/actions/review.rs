use anyhow::{bail, Result};

use crate::cli::types::ReviewAction;
use crate::db;
use crate::memory_candidate::review::{self, CandidateEdit};

pub(in crate::cli) fn run_review(action: ReviewAction) -> Result<()> {
    let mut conn = db::open_db()?;

    match action {
        ReviewAction::List { project, limit } => {
            let rows = review::list_pending(&conn, project.as_deref(), limit)?;
            if rows.is_empty() {
                println!("No pending memory candidates.");
                return Ok(());
            }
            println!("Pending memory candidates ({}):", rows.len());
            for row in rows {
                let project = row.project.as_deref().unwrap_or("<unknown project>");
                println!(
                    "  [{}] {} {} {} confidence={:.2} risk={} project={}",
                    row.id,
                    row.scope,
                    row.memory_type,
                    row.topic_key,
                    row.confidence,
                    row.risk_class,
                    project
                );
                println!("      text: {}", db::truncate_str(&row.text, 180));
                println!("      evidence: {}", row.evidence_event_ids);
                for evidence in row.evidence_preview {
                    println!("        {}", evidence);
                }
            }
        }
        ReviewAction::Approve { id } => {
            let Some(memory_id) = review::approve_candidate(&mut conn, id)? else {
                bail!("candidate {} not found", id);
            };
            println!("Approved candidate {}; promoted memory {}.", id, memory_id);
        }
        ReviewAction::Discard { id } => {
            if review::discard_candidate(&conn, id)? {
                println!("Discarded candidate {}.", id);
            } else {
                bail!("candidate {} not found or not pending_review", id);
            }
        }
        ReviewAction::Edit {
            id,
            text,
            topic_key,
            memory_type,
            scope,
        } => {
            let edit = CandidateEdit {
                scope,
                memory_type,
                topic_key,
                text,
            };
            let Some(memory_id) = review::edit_candidate(&mut conn, id, edit)? else {
                bail!("candidate {} not found", id);
            };
            println!("Edited candidate {}; promoted memory {}.", id, memory_id);
        }
    }

    Ok(())
}
