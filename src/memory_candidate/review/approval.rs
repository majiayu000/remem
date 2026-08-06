use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::json;

use crate::memory::poisoning::{scan_instruction_pattern, validate_trust_class, SourceTrustClass};
use crate::memory_candidate::{
    route_candidate, update_candidate_after_lifecycle, ParsedMemoryCandidate,
};

use super::super::apply::{promote_candidate_to_memory_with_route_and_policy, SupersedePolicy};

use super::{CandidateRow, ReviewApprovalOutcome, ReviewMeta, ReviewPromotion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PatternAcknowledgement {
    pattern_id: String,
    pattern_version: i64,
}

struct ApprovalPromotionContext {
    supersede_policy: SupersedePolicy,
    candidate_override: Option<ParsedMemoryCandidate>,
    preserve_source_payload: bool,
    dream_audit: Option<DreamApprovalAudit>,
    backfill_restore: Option<DreamBackfillRestore>,
}

/// GH-990: a stock-backfill candidate restores the exact pre-v076 memory its
/// artifacts retired, instead of promoting a fresh memory from the payload.
struct DreamBackfillRestore {
    memory_id: i64,
    project: String,
    merge_payload: super::dream_provenance::DreamMergePayload,
}

struct DreamApprovalAudit {
    review_token: String,
    artifact_ids: Vec<i64>,
    authorized_supersede_ids: Vec<i64>,
}

pub(super) fn approve_candidate_with_meta_and_ack(
    conn: &mut Connection,
    id: i64,
    meta: &ReviewMeta,
    acknowledged_pattern_id: Option<&str>,
    acknowledged_dream_review_token: Option<&str>,
) -> Result<Option<ReviewApprovalOutcome>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = approve_candidate_in_transaction(
        &tx,
        id,
        meta,
        acknowledged_pattern_id,
        acknowledged_dream_review_token,
    )?;
    tx.commit()?;
    Ok(result)
}

pub(crate) fn approve_candidate_in_transaction(
    conn: &Connection,
    id: i64,
    meta: &ReviewMeta,
    acknowledged_pattern_id: Option<&str>,
    acknowledged_dream_review_token: Option<&str>,
) -> Result<Option<ReviewApprovalOutcome>> {
    let Some(row) = super::load_candidate(conn, id)? else {
        return Ok(None);
    };
    let acknowledgement = approval_acknowledgement(&row, acknowledged_pattern_id)?;
    let promotion_context = dream_promotion_context(conn, &row, acknowledged_dream_review_token)?;
    if let Some(restore) = &promotion_context.backfill_restore {
        // GH-990: stock-backfill candidates restore the retired pre-v076
        // memory in place; nothing is promoted or superseded.
        let restored_id =
            restore_backfill_memory(conn, &row, meta, restore, acknowledgement.as_ref())?;
        if let Some(audit) = &promotion_context.dream_audit {
            persist_dream_approval_audit(conn, &row, meta, audit, &[])?;
        }
        return Ok(Some(ReviewApprovalOutcome {
            memory_id: restored_id,
            actual_superseded_ids: Vec::new(),
        }));
    }
    let promotion = promote_row(
        conn,
        &row,
        "approved",
        promotion_context.candidate_override.as_ref(),
        false,
        promotion_context.preserve_source_payload,
        meta,
        acknowledgement.as_ref(),
        promotion_context.supersede_policy,
    )?;
    let mut actual_superseded_ids = promotion.superseded_ids;
    actual_superseded_ids.sort_unstable();
    if let Some(audit) = promotion_context.dream_audit {
        persist_dream_approval_audit(conn, &row, meta, &audit, &actual_superseded_ids)?;
    }
    Ok(Some(ReviewApprovalOutcome {
        memory_id: promotion.memory_id,
        actual_superseded_ids,
    }))
}

pub(crate) fn edit_candidate_in_transaction(
    conn: &Connection,
    id: i64,
    edit: super::CandidateEdit,
    meta: &ReviewMeta,
) -> Result<Option<i64>> {
    let edit = normalize_candidate_edit(edit)?;
    let Some(row) = super::load_candidate(conn, id)? else {
        return Ok(None);
    };
    super::ensure_reviewable(&row)?;
    if row.source_kind.as_deref() == Some("dream_model_output") {
        bail!("dream_candidate_edit_unsupported");
    }
    let edited = row.apply_edit(edit)?;
    if let Some(matched) = scan_instruction_pattern(&edited.text) {
        bail!(
            "edited candidate {} matched instruction-pattern {}@v{}; review and acknowledge the pattern before promotion",
            row.id,
            matched.pattern_id,
            matched.pattern_set_version
        );
    }
    let promotion = promote_row(
        conn,
        &row,
        "edited",
        Some(&edited),
        true,
        false,
        meta,
        None,
        SupersedePolicy::Unrestricted,
    )?;
    Ok(Some(promotion.memory_id))
}

fn dream_promotion_context(
    conn: &Connection,
    row: &CandidateRow,
    acknowledged_dream_review_token: Option<&str>,
) -> Result<ApprovalPromotionContext> {
    if row.source_kind.as_deref() != Some("dream_model_output") {
        if acknowledged_dream_review_token.is_some() {
            bail!("Dream review token is only valid for Dream candidates");
        }
        return Ok(ApprovalPromotionContext {
            supersede_policy: SupersedePolicy::Unrestricted,
            candidate_override: None,
            preserve_source_payload: false,
            dream_audit: None,
            backfill_restore: None,
        });
    }
    let provenance = super::load_dream_quarantine_provenance(conn, row.id)?
        .context("Dream candidate is missing quarantine provenance")?;
    if !provenance.blocked_reasons.is_empty() {
        bail!(
            "Dream candidate provenance is not reviewable: {}",
            provenance.blocked_reasons.join(",")
        );
    }
    let approval_blocked_reasons = provenance.approval_blocked_reasons();
    if !approval_blocked_reasons.is_empty() {
        bail!(approval_blocked_reasons.join(","));
    }
    let expected_token = provenance
        .review_token
        .as_deref()
        .context("Dream candidate provenance has no review token")?;
    let acknowledged_token = acknowledged_dream_review_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("dream_provenance_ack_required")?;
    if acknowledged_token != expected_token {
        bail!("dream_provenance_changed");
    }
    let artifact_ids = provenance
        .artifacts
        .iter()
        .map(|artifact| artifact.artifact_id)
        .collect::<Vec<_>>();
    let authorized_supersede_ids = provenance.authorized_supersede_ids.clone();
    let provenance_epoch = provenance
        .artifacts
        .iter()
        .map(|artifact| artifact.created_at_epoch)
        .min()
        .context("Dream candidate provenance has no artifact timestamp")?;
    let merge_payload = provenance
        .merge_payload
        .as_ref()
        .context("Dream candidate provenance has no canonical merge payload")?;
    let backfill_restore = match provenance.backfill_memory_ids.as_slice() {
        [] => None,
        [memory_id] => Some(DreamBackfillRestore {
            memory_id: *memory_id,
            project: provenance
                .artifacts
                .first()
                .map(|artifact| artifact.project.clone())
                .context("Dream backfill provenance has no artifact project")?,
            merge_payload: merge_payload.clone(),
        }),
        _ => bail!("dream_backfill_provenance_split"),
    };
    let candidate_override = ParsedMemoryCandidate {
        scope: row.scope.clone(),
        memory_type: merge_payload.memory_type.clone(),
        topic_key: merge_payload.topic_key.clone(),
        title_override: Some(merge_payload.title.clone()),
        text: merge_payload.content.clone(),
        confidence: row.confidence,
        risk_class: row.risk_class.clone(),
    };
    Ok(ApprovalPromotionContext {
        supersede_policy: SupersedePolicy::RequireExact {
            memory_ids: provenance
                .authorized_supersede_ids
                .into_iter()
                .collect::<BTreeSet<_>>(),
            provenance_epoch,
        },
        candidate_override: Some(candidate_override),
        preserve_source_payload: true,
        dream_audit: Some(DreamApprovalAudit {
            review_token: expected_token.to_string(),
            artifact_ids,
            authorized_supersede_ids,
        }),
        backfill_restore,
    })
}

/// Restore a pre-v076 Dream-merged memory retired by the stock backfill
/// (GH-990). The artifact payload is immutable, so the row must still match
/// it exactly; any drift means another path touched the row and approval
/// fails rather than restoring the wrong content.
fn restore_backfill_memory(
    conn: &Connection,
    row: &CandidateRow,
    meta: &ReviewMeta,
    restore: &DreamBackfillRestore,
    acknowledgement: Option<&PatternAcknowledgement>,
) -> Result<i64> {
    let memory = conn
        .query_row(
            "SELECT project, memory_type, topic_key, title, content, status,
                    session_id, source_trust_class
             FROM memories WHERE id = ?1",
            params![restore.memory_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
        .context("dream_backfill_restore_target_missing")?;
    let (project, memory_type, topic_key, title, content, status, session_id, source_trust_class) =
        memory;
    if project != restore.project
        || memory_type != restore.merge_payload.memory_type
        || session_id.as_deref() != Some("dream")
        || source_trust_class != "external_content"
    {
        bail!("dream_backfill_restore_scope_mismatch");
    }
    let fallback_topic = format!("dream-backfill-{}", restore.memory_id);
    let effective_topic = topic_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&fallback_topic);
    if effective_topic != restore.merge_payload.topic_key
        || title != restore.merge_payload.title
        || content != restore.merge_payload.content
    {
        bail!("dream_backfill_restore_payload_mismatch");
    }
    if status != "archived" {
        bail!("dream_backfill_restore_target_not_archived");
    }

    let now = chrono::Utc::now().timestamp();
    let restored = conn.execute(
        "UPDATE memories SET status = 'active', updated_at_epoch = ?2
         WHERE id = ?1 AND status = 'archived'",
        params![restore.memory_id, now],
    )?;
    if restored != 1 {
        bail!("dream_backfill_restore_lost_atomicity");
    }

    let lifecycle_candidate = row.as_candidate();
    let route = row.route_for(&lifecycle_candidate);
    update_candidate_after_lifecycle(conn, row.id, &lifecycle_candidate, &route, "approved")?;
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
                restore.memory_id
            ],
        )?;
    }
    conn.execute(
        "INSERT INTO memory_operation_log
         (operation, planner_version, actor, source, owner_scope, owner_key,
          memory_type, input_topic_key, source_candidate_id, result_memory_id,
          reason, created_at_epoch)
         VALUES ('dream_backfill_restore', 'gh990-v1', ?1, 'dream_backfill',
                 'repo', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            meta.actor,
            project,
            restore.merge_payload.memory_type,
            restore.merge_payload.topic_key,
            row.id,
            restore.memory_id,
            format!(
                "restored archived stock memory on candidate {} approval",
                row.id
            ),
            now,
        ],
    )?;
    Ok(restore.memory_id)
}

fn persist_dream_approval_audit(
    conn: &Connection,
    row: &CandidateRow,
    meta: &ReviewMeta,
    audit: &DreamApprovalAudit,
    actual_superseded_ids: &[i64],
) -> Result<()> {
    let project = row
        .source_project
        .as_deref()
        .or(row.project.as_deref())
        .context("Dream candidate is missing project for approval audit")?;
    let occurred_at_epoch = chrono::Utc::now().timestamp();
    let detail = json!({
        "action": "approve",
        "actor": meta.actor,
        "action_source": meta.action_source.as_str(),
        "batch_id": meta.batch_id,
        "reason": meta.reason,
        "candidate_id": row.id,
        "dream_artifact_ids": audit.artifact_ids,
        "dream_review_token": audit.review_token,
        "authorized_supersede_ids": audit.authorized_supersede_ids,
        "actual_superseded_ids": actual_superseded_ids,
    })
    .to_string();
    let inserted = conn.execute(
        "INSERT INTO events(session_id, project, event_type, summary, detail, created_at_epoch)
         VALUES ('review:dream', ?1, 'candidate_dream_review',
                 'Dream candidate approval provenance', ?2, ?3)",
        params![project, detail, occurred_at_epoch],
    )?;
    if inserted != 1 {
        bail!("Dream candidate approval audit write lost atomicity");
    }
    Ok(())
}

pub(crate) fn normalize_candidate_edit(
    mut edit: super::CandidateEdit,
) -> Result<super::CandidateEdit> {
    if edit.scope.is_none()
        && edit.memory_type.is_none()
        && edit.topic_key.is_none()
        && edit.text.is_none()
    {
        bail!("edit requires at least one changed field");
    }
    edit.scope = edit
        .scope
        .as_deref()
        .map(crate::memory_candidate::normalize_scope)
        .transpose()?;
    edit.memory_type = edit
        .memory_type
        .as_deref()
        .map(crate::memory_candidate::normalize_memory_type)
        .transpose()?;
    edit.topic_key = edit
        .topic_key
        .as_deref()
        .map(crate::memory_candidate::normalize_topic_key)
        .transpose()?;
    if let Some(text) = edit.text.take() {
        let text = text.trim().to_string();
        if text.is_empty() {
            bail!("edit text must not be empty");
        }
        edit.text = Some(text);
    }
    Ok(edit)
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
    candidate_override: Option<&ParsedMemoryCandidate>,
    reroute_override: bool,
    preserve_source_payload: bool,
    meta: &ReviewMeta,
    acknowledgement: Option<&PatternAcknowledgement>,
    supersede_policy: SupersedePolicy,
) -> Result<ReviewPromotion> {
    let project = row
        .source_project
        .as_deref()
        .or(row.project.as_deref())
        .context("candidate is missing source project path")?;
    let candidate = candidate_override
        .cloned()
        .unwrap_or_else(|| row.as_candidate());
    let mut route = if reroute_override {
        route_candidate(project, None, &candidate, std::iter::empty())
    } else {
        row.route_for(&candidate)
    };
    if reroute_override && row.source_kind.as_deref() == Some("pack") {
        let pack_route = row.route_for(&candidate);
        route.topic_domain = pack_route.topic_domain;
        route.routing_reason = pack_route.routing_reason;
    }
    let outcome = promote_candidate_to_memory_with_route_and_policy(
        conn,
        None,
        project,
        row.id,
        &candidate,
        &row.evidence_event_ids,
        &route,
        parse_row_trust(row)?,
        supersede_policy,
    )?;
    let status = outcome.review_status_for(review_status);
    let now = chrono::Utc::now().timestamp();
    let lifecycle_candidate = if preserve_source_payload {
        row.as_candidate()
    } else {
        candidate.clone()
    };
    update_candidate_after_lifecycle(conn, row.id, &lifecycle_candidate, &route, status)?;
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
        superseded_ids: outcome.superseded_ids,
    })
}

fn parse_row_trust(row: &CandidateRow) -> Result<SourceTrustClass> {
    validate_trust_class(&row.source_trust_class)?;
    Ok(SourceTrustClass::parse(&row.source_trust_class)
        .unwrap_or(SourceTrustClass::LocalToolOutput))
}
