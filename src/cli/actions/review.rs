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
                let is_dream = row.source_kind.as_deref() == Some("dream_model_output");
                println!(
                    "  [{}] {} {} {} status={} confidence={:.2} risk={} project={} source={}",
                    row.id,
                    terminal_safe(&row.scope),
                    candidate_terminal_safe(&row.memory_type, is_dream),
                    candidate_terminal_safe(&row.topic_key, is_dream),
                    terminal_safe(&row.review_status),
                    row.confidence,
                    terminal_safe(&row.risk_class),
                    terminal_safe(project),
                    terminal_safe(row.source_kind.as_deref().unwrap_or("<unknown>"))
                );
                if let Some(pattern) = &row.quarantine_pattern_id {
                    let version = row
                        .quarantine_pattern_version
                        .map(|value| format!("@v{value}"))
                        .unwrap_or_default();
                    println!(
                        "      quarantine: {}{}",
                        terminal_safe(pattern),
                        terminal_safe(&version)
                    );
                }
                println!(
                    "      text: {}",
                    candidate_terminal_preview(&row.text, 180, is_dream)
                );
                println!("      evidence: {}", terminal_safe(&row.evidence_event_ids));
                for evidence in row.evidence_preview {
                    println!("        {}", terminal_safe(&evidence));
                }
                if let Some(provenance) = &row.dream_provenance {
                    println!(
                        "      dream_review_token: {}",
                        provenance
                            .review_token
                            .as_deref()
                            .map(terminal_safe)
                            .unwrap_or_else(|| "<unavailable>".to_string())
                    );
                    println!(
                        "      authorized_supersede_ids: {:?}",
                        provenance.authorized_supersede_ids
                    );
                    for artifact in &provenance.artifacts {
                        println!(
                            "      dream_artifact: id={} version={} occurrences={} decision={} decision_ids={:?} payload_sha256={} field={} pattern={}@v{} members={:?} intended_superseded={:?}",
                            artifact.artifact_id,
                            artifact.version,
                            artifact.occurrence_count,
                            terminal_safe(&artifact.decision_kind),
                            artifact.decision_ids,
                            terminal_safe(&artifact.decision_payload_sha256),
                            terminal_safe(&artifact.generated_field),
                            terminal_safe(&artifact.pattern_id),
                            artifact.pattern_version,
                            artifact.member_ids,
                            artifact.intended_superseded_ids
                        );
                        if let (Some(topic_key), Some(memory_type), Some(title), Some(content)) = (
                            artifact.generated_topic_key.as_deref(),
                            artifact.generated_memory_type.as_deref(),
                            artifact.generated_title.as_deref(),
                            artifact.generated_content.as_deref(),
                        ) {
                            println!(
                                "        merge_payload: topic={} type={} title={} content={}",
                                dream_terminal_safe(topic_key),
                                dream_terminal_safe(memory_type),
                                dream_terminal_preview(title, 96),
                                dream_terminal_preview(content, 180)
                            );
                        }
                    }
                    for reason in &provenance.blocked_reasons {
                        println!("      dream_blocked: {}", terminal_safe(reason));
                    }
                }
            }
        }
        ReviewAction::Approve {
            id,
            acknowledge_pattern,
            acknowledge_dream_review_token,
        } => {
            let approved = match (
                acknowledge_pattern.as_deref(),
                acknowledge_dream_review_token.as_deref(),
            ) {
                (Some(pattern), Some(review_token)) => {
                    review::approve_candidate_with_dream_ack(&mut conn, id, pattern, review_token)?
                }
                (Some(pattern), None) => {
                    review::approve_candidate_with_ack(&mut conn, id, pattern)?
                }
                (None, Some(_)) => {
                    bail!("--acknowledge-dream-review-token requires --acknowledge-pattern")
                }
                (None, None) => review::approve_candidate(&mut conn, id)?,
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
        println!("    {:<24} {:>6}", terminal_safe(memory_type), count);
    }
    println!("  By project:");
    for (project, count) in &preview.by_project {
        println!("    {:<48} {:>6}", terminal_safe(project), count);
    }
    println!("  Sample rows:");
    for sample in &preview.samples {
        println!(
            "    [{}] {} {} — {}",
            sample.id,
            terminal_safe(&sample.memory_type),
            terminal_safe(&sample.topic_key),
            terminal_safe(&sample.text)
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
        terminal_safe(&row.candidate_type),
        terminal_safe(&row.edge_type),
        terminal_safe(&row.from_ref),
        terminal_safe(&row.to_ref),
        row.confidence,
        terminal_safe(&row.risk_class),
        terminal_safe(&row.review_status),
        terminal_safe(project)
    );
    println!("      evidence: {:?}", row.evidence_event_ids);
    println!(
        "      reason: {}",
        terminal_safe(db::truncate_str(&row.reason, 180))
    );
    if let Some(edge_id) = row.promoted_edge_id {
        println!("      promoted_edge: {}", edge_id);
    }
    for evidence in &row.evidence_preview {
        println!("        {}", terminal_safe(evidence));
    }
}

fn terminal_safe(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_control() {
            output.extend(ch.escape_default());
        } else if is_bidi_format_control(ch) {
            output.extend(ch.escape_unicode());
        } else {
            output.push(ch);
        }
    }
    output
}

fn candidate_terminal_safe(value: &str, is_dream: bool) -> String {
    if is_dream {
        dream_terminal_safe(value)
    } else {
        terminal_safe(value)
    }
}

fn candidate_terminal_preview(value: &str, max_chars: usize, is_dream: bool) -> String {
    if is_dream {
        dream_terminal_preview(value, max_chars)
    } else {
        terminal_safe(db::truncate_str(value, max_chars))
    }
}

fn dream_terminal_safe(value: &str) -> String {
    terminal_safe(&crate::adapter::common::redact_sensitive_text(value))
}

fn dream_terminal_preview(value: &str, max_chars: usize) -> String {
    let redacted = crate::adapter::common::redact_sensitive_text(value);
    terminal_safe(db::truncate_str(&redacted, max_chars))
}

fn is_bidi_format_control(ch: char) -> bool {
    matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}')
        || ('\u{202a}'..='\u{202e}').contains(&ch)
        || ('\u{2066}'..='\u{2069}').contains(&ch)
}

#[cfg(test)]
mod tests {
    use super::{dream_terminal_safe, terminal_safe};

    #[test]
    fn terminal_safe_escapes_control_and_ansi_bytes_without_hiding_text() {
        let rendered = terminal_safe("visible\n\x1b[2Jspoof\r");
        assert!(rendered.contains("visible"));
        assert!(rendered.contains("spoof"));
        assert!(rendered.contains("\\n"));
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\r'));
        assert!(!rendered.contains('\x1b'));
    }

    #[test]
    fn terminal_safe_escapes_unicode_bidi_format_controls() {
        let rendered = terminal_safe("left\u{202e}right\u{2066}end");
        assert_eq!(rendered, "left\\u{202e}right\\u{2066}end");
        assert!(!rendered.contains('\u{202e}'));
        assert!(!rendered.contains('\u{2066}'));
    }

    #[test]
    fn dream_terminal_safe_redacts_secrets_before_escaping_controls() {
        let secret = "ghp_1234567890abcdef";
        let rendered = dream_terminal_safe(&format!("visible {secret}\n\x1b[2J\u{202e}spoof"));
        assert!(rendered.contains("visible [REDACTED]"));
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\x1b'));
        assert!(!rendered.contains('\u{202e}'));
        assert!(rendered.contains("\\n"));
        assert!(rendered.contains("\\u{202e}"));
    }
}
