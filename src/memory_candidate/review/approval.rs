use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};

use crate::memory::poisoning::{validate_trust_class, SourceTrustClass};
use crate::memory_candidate::{
    promote_candidate_to_memory_with_route, route_candidate, update_candidate_after_lifecycle,
    ParsedMemoryCandidate,
};

use super::{CandidateRow, ReviewMeta, ReviewPromotion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PatternAcknowledgement {
    pattern_id: String,
    pattern_version: i64,
}

pub(super) fn approve_candidate_with_meta_and_ack(
    conn: &mut Connection,
    id: i64,
    meta: &ReviewMeta,
    acknowledged_pattern_id: Option<&str>,
) -> Result<Option<i64>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(row) = super::load_candidate(&tx, id)? else {
        return Ok(None);
    };
    let acknowledgement = approval_acknowledgement(&row, acknowledged_pattern_id)?;
    let promotion = promote_row(&tx, &row, "approved", None, meta, acknowledgement.as_ref())?;
    tx.commit()?;
    Ok(Some(promotion.memory_id))
}

fn approval_acknowledgement(
    row: &CandidateRow,
    acknowledged_pattern_id: Option<&str>,
) -> Result<Option<PatternAcknowledgement>> {
    match row.review_status.as_str() {
        "pending_review" => {
            if acknowledged_pattern_id.is_some() {
                bail!(
                    "candidate {} is pending_review; acknowledge-pattern is only valid for quarantined candidates",
                    row.id
                );
            }
            Ok(None)
        }
        "quarantined" => {
            let expected_pattern = row
                .quarantine_pattern_id
                .as_deref()
                .context("quarantined candidate is missing quarantine_pattern_id")?;
            let expected_version = row
                .quarantine_pattern_version
                .context("quarantined candidate is missing quarantine_pattern_version")?;
            let Some(acknowledged_pattern_id) = acknowledged_pattern_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                bail!(
                    "candidate {} is quarantined by pattern {}; pass --acknowledge-pattern {} to approve after review",
                    row.id,
                    expected_pattern,
                    expected_pattern
                );
            };
            if acknowledged_pattern_id != expected_pattern {
                bail!(
                    "candidate {} acknowledged pattern {} does not match quarantine pattern {}",
                    row.id,
                    acknowledged_pattern_id,
                    expected_pattern
                );
            }
            Ok(Some(PatternAcknowledgement {
                pattern_id: expected_pattern.to_string(),
                pattern_version: expected_version,
            }))
        }
        _ => {
            super::ensure_pending(row)?;
            Ok(None)
        }
    }
}

pub(super) fn promote_row(
    conn: &Connection,
    row: &CandidateRow,
    review_status: &str,
    edited: Option<&ParsedMemoryCandidate>,
    meta: &ReviewMeta,
    acknowledgement: Option<&PatternAcknowledgement>,
) -> Result<ReviewPromotion> {
    let project = row
        .source_project
        .as_deref()
        .or(row.project.as_deref())
        .context("candidate is missing source project path")?;
    let candidate = edited.cloned().unwrap_or_else(|| row.as_candidate());
    let mut route = if edited.is_some() {
        route_candidate(project, None, &candidate, std::iter::empty())
    } else {
        row.route_for(&candidate)
    };
    if edited.is_some() && row.source_kind.as_deref() == Some("pack") {
        let pack_route = row.route_for(&candidate);
        route.topic_domain = pack_route.topic_domain;
        route.routing_reason = pack_route.routing_reason;
    }
    let outcome = promote_candidate_to_memory_with_route(
        conn,
        None,
        project,
        row.id,
        &candidate,
        &row.evidence_event_ids,
        &route,
        parse_row_trust(row)?,
    )?;
    let status = outcome.review_status_for(review_status);
    let now = chrono::Utc::now().timestamp();
    update_candidate_after_lifecycle(conn, row.id, &candidate, &route, status)?;
    conn.execute(
        "UPDATE memory_candidates
         SET updated_at_epoch = ?1, review_actor = ?2, reviewed_at_epoch = ?1,
             review_action_source = ?3, review_batch_id = ?4, review_reason = ?5
         WHERE id = ?6",
        params![
            now,
            meta.actor,
            meta.action_source.as_str(),
            meta.batch_id,
            meta.reason,
            row.id
        ],
    )?;
    let memory_id = outcome
        .memory_id
        .context("candidate promotion produced no memory id")?;
    if let Some(acknowledgement) = acknowledgement {
        conn.execute(
            "UPDATE memory_candidates
             SET acknowledged_pattern_id = ?1, acknowledged_pattern_version = ?2,
                 acknowledged_at_epoch = ?3, updated_at_epoch = ?3
             WHERE id = ?4",
            params![
                acknowledgement.pattern_id.as_str(),
                acknowledgement.pattern_version,
                now,
                row.id
            ],
        )?;
        conn.execute(
            "UPDATE memories
             SET acknowledged_pattern_id = ?1, acknowledged_pattern_version = ?2,
                 acknowledged_at_epoch = ?3
             WHERE id = ?4",
            params![
                acknowledgement.pattern_id.as_str(),
                acknowledgement.pattern_version,
                now,
                memory_id
            ],
        )?;
    }
    Ok(ReviewPromotion {
        memory_id,
        promoted: outcome.promoted,
    })
}

fn parse_row_trust(row: &CandidateRow) -> Result<SourceTrustClass> {
    validate_trust_class(&row.source_trust_class)?;
    Ok(SourceTrustClass::parse(&row.source_trust_class)
        .unwrap_or(SourceTrustClass::LocalToolOutput))
}
