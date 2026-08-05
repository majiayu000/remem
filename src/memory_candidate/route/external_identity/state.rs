use anyhow::{bail, Result};
use rusqlite::{Connection, OptionalExtension};

use super::ExternalCandidateDisposition;
use crate::memory::poisoning::InstructionPatternMatch;

#[derive(Debug)]
pub(super) struct ExistingExternalCandidate {
    pub(super) id: i64,
    review_status: String,
    review_action_source: Option<String>,
    quarantine_pattern_id: Option<String>,
    quarantine_pattern_version: Option<i64>,
    acknowledged_pattern_id: Option<String>,
    acknowledged_pattern_version: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExistingCandidateAction {
    Duplicate,
    QuarantinePending(InstructionPatternMatch),
    RefreshQuarantine(InstructionPatternMatch),
    RecordTerminalDuplicate,
    RecordAcknowledgedPattern(InstructionPatternMatch),
    RecordDiscardedPattern(InstructionPatternMatch),
    CreateReviewCandidate(InstructionPatternMatch),
}

impl ExistingCandidateAction {
    pub(super) fn disposition(self) -> ExternalCandidateDisposition {
        match self {
            Self::CreateReviewCandidate(_) => ExternalCandidateDisposition::Quarantined,
            _ => ExternalCandidateDisposition::Duplicate,
        }
    }
}

pub(super) fn load_and_validate_candidate(
    conn: &Connection,
    candidate_id: i64,
    expected_source_kind: &str,
    expected_memory_type: &str,
) -> Result<ExistingExternalCandidate> {
    let row = conn
        .query_row(
            "SELECT source_kind, memory_type, source_trust_class, review_status,
                    review_action_source,
                    quarantine_pattern_id, quarantine_pattern_version,
                    acknowledged_pattern_id, acknowledged_pattern_version
             FROM memory_candidates WHERE id = ?1",
            [candidate_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        source_kind,
        memory_type,
        trust,
        status,
        review_action_source,
        pattern_id,
        pattern_version,
        ack_id,
        ack_version,
    )) = row
    else {
        bail!("external candidate identity references missing candidate id={candidate_id}");
    };
    if source_kind.as_deref() != Some(expected_source_kind)
        || (memory_type != expected_memory_type && status != "edited")
        || trust != "external_content"
    {
        bail!(
            "external candidate identity source contract mismatch for candidate id={candidate_id}"
        );
    }
    let pattern_pair_valid = pattern_id.is_some() == pattern_version.is_some()
        && pattern_version.is_none_or(|version| version > 0);
    let ack_pair_valid =
        ack_id.is_some() == ack_version.is_some() && ack_version.is_none_or(|version| version > 0);
    if !pattern_pair_valid || !ack_pair_valid {
        bail!("external candidate pattern contract mismatch for candidate id={candidate_id}");
    }
    match status.as_str() {
        "pending_review" if pattern_id.is_none() && ack_id.is_none() => {}
        "quarantined" if pattern_id.is_some() && ack_id.is_none() => {}
        "approved" | "edited" | "discarded" | "noop" => {}
        "pending_review" | "quarantined" => bail!(
            "external candidate review-state pattern mismatch for candidate id={candidate_id}"
        ),
        _ => bail!(
            "external candidate identity has unsupported review status for candidate id={candidate_id}"
        ),
    }
    Ok(ExistingExternalCandidate {
        id: candidate_id,
        review_status: status,
        review_action_source,
        quarantine_pattern_id: pattern_id,
        quarantine_pattern_version: pattern_version,
        acknowledged_pattern_id: ack_id,
        acknowledged_pattern_version: ack_version,
    })
}

pub(super) fn validate_quarantine_match(matched: Option<InstructionPatternMatch>) -> Result<()> {
    if matched.is_some_and(|matched| {
        matched.pattern_id.trim().is_empty() || matched.pattern_set_version <= 0
    }) {
        bail!("external candidate quarantine pattern contract is invalid");
    }
    Ok(())
}

pub(super) fn existing_candidate_action(
    candidate: &ExistingExternalCandidate,
    quarantine_match: Option<InstructionPatternMatch>,
) -> ExistingCandidateAction {
    match candidate.review_status.as_str() {
        "pending_review" => quarantine_match
            .map_or(ExistingCandidateAction::Duplicate, |matched| {
                ExistingCandidateAction::QuarantinePending(matched)
            }),
        "quarantined" => quarantine_match.map_or(ExistingCandidateAction::Duplicate, |matched| {
            if candidate.quarantine_pattern_id.as_deref() == Some(matched.pattern_id)
                && candidate.quarantine_pattern_version == Some(matched.pattern_set_version)
            {
                ExistingCandidateAction::Duplicate
            } else {
                ExistingCandidateAction::RefreshQuarantine(matched)
            }
        }),
        _ => {
            let Some(matched) = quarantine_match else {
                return ExistingCandidateAction::RecordTerminalDuplicate;
            };
            if candidate.acknowledged_pattern_id.as_deref() == Some(matched.pattern_id)
                && candidate.acknowledged_pattern_version == Some(matched.pattern_set_version)
            {
                ExistingCandidateAction::RecordAcknowledgedPattern(matched)
            } else if candidate.review_status == "discarded"
                && candidate.quarantine_pattern_id.as_deref() == Some(matched.pattern_id)
                && candidate.quarantine_pattern_version == Some(matched.pattern_set_version)
            {
                if candidate.review_action_source.as_deref() == Some("dream_semantic_superseded") {
                    ExistingCandidateAction::CreateReviewCandidate(matched)
                } else {
                    ExistingCandidateAction::RecordDiscardedPattern(matched)
                }
            } else {
                ExistingCandidateAction::CreateReviewCandidate(matched)
            }
        }
    }
}
