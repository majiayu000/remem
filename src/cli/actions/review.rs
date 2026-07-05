use anyhow::{bail, Result};

use crate::cli::types::{GraphReviewAction, ReviewAction, ReviewBatchFilterArgs};
use crate::db;
use crate::graph_candidate::review as graph_review;
use crate::memory_candidate::review::{self, BatchFilter, BatchPreview, CandidateEdit, ReviewMeta};
use crate::memory_candidate::review_stats;

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
        ReviewAction::Approve {
            id,
            acknowledge_pattern,
        } => {
            let approved = match acknowledge_pattern.as_deref() {
                Some(pattern) => review::approve_candidate_with_ack(&mut conn, id, pattern)?,
                None => review::approve_candidate(&mut conn, id)?,
            };
            let Some(memory_id) = approved else {
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
        ReviewAction::ApproveBatch { filter, yes } => {
            let filter = batch_filter_from_args(filter);
            let preview = review::resolve_batch(&conn, &filter)?;
            if preview.ids.is_empty() {
                println!("No pending candidates match the filters.");
                return Ok(());
            }
            print_batch_preview("approve", &preview);
            if !yes && !confirm_batch()? {
                println!("Aborted; no candidates were changed.");
                return Ok(());
            }
            let meta =
                ReviewMeta::batch(review::default_review_actor(), review::new_batch_id(), None);
            let outcome = review::approve_batch(&mut conn, &preview, &meta)?;
            println!(
                "Approved {} candidate(s); promoted {} memory(ies). batch_id={}",
                outcome.processed.len(),
                outcome.promoted_memory_ids.len(),
                outcome.batch_id
            );
        }
        ReviewAction::DiscardBatch {
            filter,
            reason,
            yes,
        } => {
            let filter = batch_filter_from_args(filter);
            let preview = review::resolve_batch(&conn, &filter)?;
            if preview.ids.is_empty() {
                println!("No pending candidates match the filters.");
                return Ok(());
            }
            print_batch_preview("discard", &preview);
            if !yes && !confirm_batch()? {
                println!("Aborted; no candidates were changed.");
                return Ok(());
            }
            let meta = ReviewMeta::batch(
                review::default_review_actor(),
                review::new_batch_id(),
                reason,
            );
            let outcome = review::discard_batch(&mut conn, &preview, &meta)?;
            println!(
                "Discarded {} candidate(s). batch_id={}",
                outcome.processed.len(),
                outcome.batch_id
            );
        }
        ReviewAction::Blocked { project } => {
            let reasons = review_stats::query_block_reasons(&conn, project.as_deref())?;
            if reasons.is_empty() {
                println!("No pending candidates.");
                return Ok(());
            }
            println!("Pending candidates by block reason:");
            for reason in reasons {
                println!(
                    "  {:<48} {:>6}  examples: {}",
                    reason.reason.as_deref().unwrap_or("<none>"),
                    reason.pending,
                    reason
                        .example_ids
                        .iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }

    Ok(())
}

fn batch_filter_from_args(args: ReviewBatchFilterArgs) -> BatchFilter {
    BatchFilter {
        project: args.project,
        memory_type: args.memory_type,
        block_reason: args.block_reason,
        topic_key: args.topic_key,
        contains: args.contains,
        min_confidence: args.min_confidence,
        older_than_days: args.older_than_days,
        limit: args.limit,
    }
}

fn print_batch_preview(action: &str, preview: &BatchPreview) {
    println!(
        "Batch {} preview: {} candidate(s) match.",
        action,
        preview.ids.len()
    );
    println!("  By type:");
    for (memory_type, count) in &preview.by_type {
        println!("    {:<24} {:>6}", memory_type, count);
    }
    println!("  By project:");
    for (project, count) in &preview.by_project {
        println!("    {:<48} {:>6}", project, count);
    }
    println!("  Sample rows:");
    for sample in &preview.samples {
        println!(
            "    [{}] {} {} — {}",
            sample.id, sample.memory_type, sample.topic_key, sample.text
        );
    }
}

fn confirm_batch() -> Result<bool> {
    use std::io::{BufRead, IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        bail!("refusing to run a batch without confirmation on a non-interactive stdin; pass --yes to proceed");
    }
    print!("Proceed? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer == "y" || answer == "yes")
}

pub(in crate::cli) fn run_graph_review(action: GraphReviewAction) -> Result<()> {
    let mut conn = db::open_db()?;

    match action {
        GraphReviewAction::List { project, limit } => {
            let rows = graph_review::list_pending(&conn, project.as_deref(), limit)?;
            if rows.is_empty() {
                println!("No reviewable graph candidates.");
                return Ok(());
            }
            println!("Reviewable graph candidates ({}):", rows.len());
            for row in rows {
                print_graph_candidate(&row);
            }
        }
        GraphReviewAction::Inspect { id } => {
            let Some(row) = graph_review::inspect_candidate(&conn, id)? else {
                bail!("graph candidate {} not found", id);
            };
            print_graph_candidate(&row);
        }
        GraphReviewAction::Approve { id } => {
            let Some(edge_id) = graph_review::approve_candidate(&mut conn, id)? else {
                bail!("graph candidate {} not found", id);
            };
            println!(
                "Approved graph candidate {}; promoted graph edge {}.",
                id, edge_id
            );
        }
        GraphReviewAction::Reject { id, reason } => {
            if graph_review::reject_candidate(&conn, id, &reason)? {
                println!("Rejected graph candidate {}.", id);
            } else {
                bail!("graph candidate {} not found or not reviewable", id);
            }
        }
        GraphReviewAction::Defer { id, reason } => {
            if graph_review::defer_candidate(&conn, id, &reason)? {
                println!("Deferred graph candidate {}.", id);
            } else {
                bail!("graph candidate {} not found or not reviewable", id);
            }
        }
    }

    Ok(())
}

fn print_graph_candidate(row: &graph_review::ReviewGraphCandidate) {
    let project = row.project.as_deref().unwrap_or("<unknown project>");
    println!(
        "  [{}] {} {} {} -> {} confidence={:.2} risk={} status={} project={}",
        row.id,
        row.candidate_type,
        row.edge_type,
        row.from_ref,
        row.to_ref,
        row.confidence,
        row.risk_class,
        row.review_status,
        project
    );
    println!("      evidence: {:?}", row.evidence_event_ids);
    println!("      reason: {}", db::truncate_str(&row.reason, 180));
    if let Some(edge_id) = row.promoted_edge_id {
        println!("      promoted_edge: {}", edge_id);
    }
    for evidence in &row.evidence_preview {
        println!("        {}", evidence);
    }
}
