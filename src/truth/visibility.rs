//! Deterministic read-only trust/visibility classification for memories.
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
pub const CURRENT_CONFIDENCE_FLOOR: f64 = 0.80;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryVisibilityClass {
    Current,
    LegacyUnverified,
    Quarantined,
    Expired,
    Superseded,
    NotYetValid,
    Inactive,
}
impl MemoryVisibilityClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::LegacyUnverified => "legacy_unverified",
            Self::Quarantined => "quarantined",
            Self::Expired => "expired",
            Self::Superseded => "superseded",
            Self::NotYetValid => "not_yet_valid",
            Self::Inactive => "inactive",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryVisibilityReason {
    CurrentEligible,
    StatusQuarantined,
    ValidityExpired,
    StatusSuperseded,
    ValidityNotYetStarted,
    StatusInactive,
    ProvenanceMissing,
    ProvenanceMalformed,
    ConfidenceMissing,
    ConfidenceBelowFloor,
    ValidityStartMissing,
    MutableStateIdentityMissing,
    RowMissing,
}
impl MemoryVisibilityReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CurrentEligible => "current_eligible",
            Self::StatusQuarantined => "status_quarantined",
            Self::ValidityExpired => "validity_expired",
            Self::StatusSuperseded => "status_superseded",
            Self::ValidityNotYetStarted => "validity_not_yet_started",
            Self::StatusInactive => "status_inactive",
            Self::ProvenanceMissing => "legacy_unverified_provenance_missing",
            Self::ProvenanceMalformed => "legacy_unverified_provenance_malformed",
            Self::ConfidenceMissing => "legacy_unverified_confidence_missing",
            Self::ConfidenceBelowFloor => "legacy_unverified_confidence_below_floor",
            Self::ValidityStartMissing => "legacy_unverified_validity_start_missing",
            Self::MutableStateIdentityMissing => "legacy_unverified_mutable_state_identity_missing",
            Self::RowMissing => "legacy_unverified_row_missing",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MemoryVisibility {
    pub classification: MemoryVisibilityClass,
    pub reason: MemoryVisibilityReason,
    pub current_context_eligible: bool,
}
impl MemoryVisibility {
    fn excluded(classification: MemoryVisibilityClass, reason: MemoryVisibilityReason) -> Self {
        Self {
            classification,
            reason,
            current_context_eligible: false,
        }
    }
}
struct Row {
    status: String,
    memory_type: String,
    topic_key: Option<String>,
    source_candidate_id: Option<i64>,
    evidence_event_ids: Option<String>,
    source_trust_class: Option<String>,
    confidence: Option<f64>,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    expires_at_epoch: Option<i64>,
    state_key_id: Option<i64>,
    lesson_has_proof: bool,
    candidate_has_proof: bool,
    direct_evidence_resolves: bool,
    state_key_resolves: bool,
}
pub fn classify_memory(
    conn: &Connection,
    memory_id: i64,
    as_of_epoch: i64,
) -> Result<MemoryVisibility> {
    let row = conn
        .query_row(
            "SELECT status, memory_type, topic_key, source_candidate_id, evidence_event_ids,
                source_trust_class,
                COALESCE(confidence, (
                    SELECT candidate.confidence FROM memory_candidates candidate
                    WHERE candidate.id = memories.source_candidate_id
                      AND candidate.review_status IN ('accepted', 'approved')
                )),
                COALESCE(valid_from_epoch, (
                    SELECT candidate.created_at_epoch FROM memory_candidates candidate
                    WHERE candidate.id = memories.source_candidate_id
                      AND candidate.review_status IN ('accepted', 'approved')
                )), valid_to_epoch,
                expires_at_epoch, state_key_id,
                EXISTS(
                    SELECT 1 FROM memory_lessons lesson
                    WHERE lesson.memory_id = memories.id
                      AND lesson.confidence >= 0.80
                      AND trim(COALESCE(lesson.source_evidence, '')) <> ''
                ),
                EXISTS(
                    SELECT 1 FROM memory_candidates candidate
                    WHERE candidate.id = memories.source_candidate_id
                      AND candidate.review_status IN ('accepted', 'approved')
                      AND CASE WHEN json_valid(candidate.evidence_event_ids) THEN
                          json_array_length(candidate.evidence_event_ids) > 0
                          AND NOT EXISTS (
                              SELECT 1 FROM json_each(candidate.evidence_event_ids) evidence
                              LEFT JOIN captured_events event ON event.id = evidence.value
                              WHERE evidence.type <> 'integer' OR event.id IS NULL
                          )
                      ELSE 0 END
                ),
                CASE
                    WHEN evidence_event_ids IS NULL THEN 1
                    WHEN json_valid(evidence_event_ids) THEN
                        json_array_length(evidence_event_ids) > 0
                        AND NOT EXISTS (
                            SELECT 1 FROM json_each(evidence_event_ids) evidence
                            LEFT JOIN captured_events event ON event.id = evidence.value
                            WHERE evidence.type <> 'integer' OR event.id IS NULL
                        )
                    ELSE 0
                END,
                EXISTS(SELECT 1 FROM memory_state_keys state_key
                       WHERE state_key.id = memories.state_key_id)
         FROM memories WHERE id = ?1",
            [memory_id],
            |r| {
                Ok(Row {
                    status: r.get(0)?,
                    memory_type: r.get(1)?,
                    topic_key: r.get(2)?,
                    source_candidate_id: r.get(3)?,
                    evidence_event_ids: r.get(4)?,
                    source_trust_class: r.get(5)?,
                    confidence: r.get(6)?,
                    valid_from_epoch: r.get(7)?,
                    valid_to_epoch: r.get(8)?,
                    expires_at_epoch: r.get(9)?,
                    state_key_id: r.get(10)?,
                    lesson_has_proof: r.get(11)?,
                    candidate_has_proof: r.get(12)?,
                    direct_evidence_resolves: r.get(13)?,
                    state_key_resolves: r.get(14)?,
                })
            },
        )
        .optional()
        .context("classify memory trust and visibility")?;
    Ok(row.map_or_else(
        || {
            MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::RowMissing,
            )
        },
        |row| classify_row(&row, as_of_epoch),
    ))
}

/// Shared G2 gate for every current-context reader.
///
/// SessionStart, UserPromptSubmit, `current_state`, and recall must call this
/// instead of treating `status='active'` as current.
pub fn admit_for_current_context(
    conn: &Connection,
    memory_id: i64,
    as_of_epoch: i64,
) -> Result<MemoryVisibility> {
    classify_memory(conn, memory_id, as_of_epoch)
}

fn classify_row(row: &Row, as_of: i64) -> MemoryVisibility {
    if row.status == "quarantined" {
        return MemoryVisibility::excluded(
            MemoryVisibilityClass::Quarantined,
            MemoryVisibilityReason::StatusQuarantined,
        );
    }
    if row.valid_to_epoch.is_some_and(|v| v <= as_of)
        || row.expires_at_epoch.is_some_and(|v| v <= as_of)
    {
        return MemoryVisibility::excluded(
            MemoryVisibilityClass::Expired,
            MemoryVisibilityReason::ValidityExpired,
        );
    }
    if row.status == "superseded" {
        return MemoryVisibility::excluded(
            MemoryVisibilityClass::Superseded,
            MemoryVisibilityReason::StatusSuperseded,
        );
    }
    if row.valid_from_epoch.is_some_and(|v| v > as_of) {
        return MemoryVisibility::excluded(
            MemoryVisibilityClass::NotYetValid,
            MemoryVisibilityReason::ValidityNotYetStarted,
        );
    }
    if row.status != "active" {
        return MemoryVisibility::excluded(
            MemoryVisibilityClass::Inactive,
            MemoryVisibilityReason::StatusInactive,
        );
    }
    let direct_user = row.source_trust_class.as_deref() == Some("user_prompt");
    let explicit_writer_proof = direct_user || row.lesson_has_proof;
    if !explicit_writer_proof {
        if row.source_candidate_id.is_none() && row.evidence_event_ids.is_none() {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ProvenanceMissing,
            );
        }
        let valid_evidence = row.evidence_event_ids.as_deref().is_none_or(|raw| {
            serde_json::from_str::<Vec<i64>>(raw)
                .ok()
                .is_some_and(|ids| !ids.is_empty() && ids.into_iter().all(|id| id > 0))
        });
        if !valid_evidence {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ProvenanceMalformed,
            );
        }
        if row.source_candidate_id.is_some() && !row.candidate_has_proof {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ProvenanceMalformed,
            );
        }
        if row.evidence_event_ids.is_some() && !row.direct_evidence_resolves {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ProvenanceMalformed,
            );
        }
        let Some(confidence) = row.confidence else {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ConfidenceMissing,
            );
        };
        if !confidence.is_finite() || confidence < CURRENT_CONFIDENCE_FLOOR {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ConfidenceBelowFloor,
            );
        }
        if row.valid_from_epoch.is_none() {
            return MemoryVisibility::excluded(
                MemoryVisibilityClass::LegacyUnverified,
                MemoryVisibilityReason::ValidityStartMissing,
            );
        }
    }
    let mutable = matches!(
        row.memory_type.as_str(),
        "decision" | "architecture" | "preference"
    ) && row
        .topic_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    if !direct_user && mutable && (row.state_key_id.is_none() || !row.state_key_resolves) {
        return MemoryVisibility::excluded(
            MemoryVisibilityClass::LegacyUnverified,
            MemoryVisibilityReason::MutableStateIdentityMissing,
        );
    }
    MemoryVisibility {
        classification: MemoryVisibilityClass::Current,
        reason: MemoryVisibilityReason::CurrentEligible,
        current_context_eligible: true,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn row() -> Row {
        Row {
            status: "active".into(),
            memory_type: "bugfix".into(),
            topic_key: None,
            source_candidate_id: Some(1),
            evidence_event_ids: Some("[2]".into()),
            source_trust_class: Some("local_tool_output".into()),
            confidence: Some(0.9),
            valid_from_epoch: Some(10),
            valid_to_epoch: None,
            expires_at_epoch: None,
            state_key_id: None,
            lesson_has_proof: false,
            candidate_has_proof: true,
            direct_evidence_resolves: true,
            state_key_resolves: true,
        }
    }
    #[test]
    fn unknown_proof_fails_closed() {
        let mut value = row();
        value.confidence = None;
        assert_eq!(
            classify_row(&value, 20).reason.as_str(),
            "legacy_unverified_confidence_missing"
        );
    }
    #[test]
    fn lifecycle_precedes_proof_and_current_rows_survive() {
        assert!(classify_row(&row(), 20).current_context_eligible);
        let mut value = row();
        value.status = "quarantined".into();
        value.source_candidate_id = None;
        assert_eq!(
            classify_row(&value, 20).classification,
            MemoryVisibilityClass::Quarantined
        );
    }

    #[test]
    fn lifecycle_collision_order_and_future_validity_are_stable() {
        let mut value = row();
        value.status = "superseded".into();
        value.expires_at_epoch = Some(20);
        assert_eq!(
            classify_row(&value, 20).classification,
            MemoryVisibilityClass::Expired
        );
        value.expires_at_epoch = None;
        value.valid_from_epoch = Some(21);
        assert_eq!(
            classify_row(&value, 20).reason,
            MemoryVisibilityReason::StatusSuperseded
        );
        value.status = "active".into();
        assert_eq!(
            classify_row(&value, 20).reason,
            MemoryVisibilityReason::ValidityNotYetStarted
        );
    }
    #[test]
    fn direct_user_proof_preserves_compatibility() {
        let mut value = row();
        value.source_trust_class = Some("user_prompt".into());
        value.source_candidate_id = None;
        value.evidence_event_ids = None;
        value.confidence = None;
        value.valid_from_epoch = None;
        assert!(classify_row(&value, 20).current_context_eligible);
    }

    #[test]
    fn direct_user_preference_with_writer_state_identity_is_current() {
        let mut value = row();
        value.memory_type = "preference".into();
        value.topic_key = Some("manual-formatting".into());
        value.state_key_id = None;
        value.source_trust_class = Some("user_prompt".into());
        value.source_candidate_id = None;
        value.evidence_event_ids = None;
        value.confidence = None;
        value.valid_from_epoch = None;
        assert!(classify_row(&value, 20).current_context_eligible);
    }

    #[test]
    fn dangling_candidate_and_event_references_fail_closed() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_candidate_id, confidence, valid_from_epoch)
             VALUES (1, '/repo', 'dangling candidate', 'body', 'bugfix',
                     1, 1, 'active', 999, 0.9, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, evidence_event_ids, confidence, valid_from_epoch)
             VALUES (2, '/repo', 'dangling event', 'body', 'bugfix',
                     1, 1, 'active', '[999]', 0.9, 1)",
            [],
        )?;

        for id in [1, 2] {
            let visibility = classify_memory(&conn, id, 2)?;
            assert_eq!(
                visibility.reason,
                MemoryVisibilityReason::ProvenanceMalformed
            );
            assert!(!visibility.current_context_eligible);
        }
        Ok(())
    }

    #[test]
    fn syntactically_malformed_candidate_and_direct_evidence_fail_closed() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memory_candidates
             (id, scope, memory_type, topic_key, text, evidence_event_ids, confidence,
              risk_class, review_status, created_at_epoch, updated_at_epoch)
             VALUES (1, 'project', 'bugfix', 'malformed-proof', 'candidate', 'not-json', 0.9,
                     'low', 'accepted', 1, 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_candidate_id)
             VALUES (1, '/repo', 'bad candidate evidence', 'body', 'bugfix',
                     1, 1, 'active', 1)",
            [],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, evidence_event_ids, confidence, valid_from_epoch)
             VALUES (2, '/repo', 'bad direct evidence', 'body', 'bugfix',
                     1, 1, 'active', 'not-json', 0.9, 1)",
            [],
        )?;

        for id in [1, 2] {
            let visibility = classify_memory(&conn, id, 2)?;
            assert_eq!(
                visibility.reason,
                MemoryVisibilityReason::ProvenanceMalformed
            );
            assert!(!visibility.current_context_eligible);
        }
        Ok(())
    }
}
