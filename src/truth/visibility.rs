//! Deterministic read-only trust/visibility classification for memories.
use anyhow::{Context, Result};
use rusqlite::{params_from_iter, Connection, Row as SqlRow};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
pub const CURRENT_CONFIDENCE_FLOOR: f64 = 0.80;
/// SQLite caps bound parameters per statement; stay well under it so a large
/// context load still classifies in a handful of statements.
const VISIBILITY_BATCH_CHUNK_SIZE: usize = 900;
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

    /// True when a row the classifier excluded was admitted anyway because the
    /// gate is running in shadow mode. Consumers use this to report what
    /// enforcement *would* have dropped.
    pub fn admitted_by_shadow_mode(&self) -> bool {
        self.current_context_eligible && self.classification != MemoryVisibilityClass::Current
    }
}

/// Rollout control for the G2 current-context gate.
///
/// Enforcement changes which memories reach live context, and on databases with
/// a large pre-provenance history it can exclude nearly the whole active set. An
/// operator needs a way to measure that before and after flipping it, and a way
/// back if injection collapses in production.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentContextGateMode {
    /// Classify and exclude non-current rows. Default.
    Enforce,
    /// Classify and report, but still admit rows that are only excluded for
    /// `legacy_unverified` reasons.
    ///
    /// Lifecycle exclusions (quarantined, expired, superseded, not-yet-valid,
    /// inactive) are pre-existing security and correctness boundaries, not part
    /// of this rollout, so shadow mode never relaxes them.
    Shadow,
}

pub const CURRENT_CONTEXT_GATE_ENV: &str = "REMEM_CURRENT_CONTEXT_GATE";

/// Read the gate mode from [`CURRENT_CONTEXT_GATE_ENV`].
///
/// Unset or unrecognized values fail closed to [`CurrentContextGateMode::Enforce`].
pub fn current_context_gate_mode() -> CurrentContextGateMode {
    match std::env::var(CURRENT_CONTEXT_GATE_ENV)
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("shadow") => CurrentContextGateMode::Shadow,
        _ => CurrentContextGateMode::Enforce,
    }
}

fn apply_gate_mode(visibility: MemoryVisibility) -> MemoryVisibility {
    if visibility.current_context_eligible
        || visibility.classification != MemoryVisibilityClass::LegacyUnverified
        || current_context_gate_mode() != CurrentContextGateMode::Shadow
    {
        return visibility;
    }
    MemoryVisibility {
        current_context_eligible: true,
        ..visibility
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
const VISIBILITY_PROJECTION_SQL: &str =
    "SELECT id, status, memory_type, topic_key, source_candidate_id, evidence_event_ids,
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
         FROM memories WHERE id IN";

fn read_visibility_row(row: &SqlRow<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        status: row.get(1)?,
        memory_type: row.get(2)?,
        topic_key: row.get(3)?,
        source_candidate_id: row.get(4)?,
        evidence_event_ids: row.get(5)?,
        source_trust_class: row.get(6)?,
        confidence: row.get(7)?,
        valid_from_epoch: row.get(8)?,
        valid_to_epoch: row.get(9)?,
        expires_at_epoch: row.get(10)?,
        state_key_id: row.get(11)?,
        lesson_has_proof: row.get(12)?,
        candidate_has_proof: row.get(13)?,
        direct_evidence_resolves: row.get(14)?,
        state_key_resolves: row.get(15)?,
    })
}

fn missing_row_visibility() -> MemoryVisibility {
    MemoryVisibility::excluded(
        MemoryVisibilityClass::LegacyUnverified,
        MemoryVisibilityReason::RowMissing,
    )
}

/// Classify many memories in chunked statements.
///
/// Every requested id is present in the result. Ids with no `memories` row stay
/// at the fail-closed `RowMissing` default rather than being omitted, so callers
/// cannot mistake an absent key for an admitted memory.
pub fn classify_memories(
    conn: &Connection,
    memory_ids: &[i64],
    as_of_epoch: i64,
) -> Result<HashMap<i64, MemoryVisibility>> {
    let ids = memory_ids.iter().copied().collect::<BTreeSet<_>>();
    let mut classifications = ids
        .iter()
        .map(|id| (*id, missing_row_visibility()))
        .collect::<HashMap<_, _>>();
    for chunk in ids
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .chunks(VISIBILITY_BATCH_CHUNK_SIZE)
    {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("{VISIBILITY_PROJECTION_SQL} ({placeholders})");
        let mut statement = conn
            .prepare(&sql)
            .context("prepare batch memory trust and visibility classification")?;
        let rows = statement
            .query_map(params_from_iter(chunk.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, read_visibility_row(row)?))
            })
            .context("query batch memory trust and visibility classification")?;
        for row in rows {
            let (id, visibility_row) =
                row.context("read batch memory trust and visibility classification row")?;
            classifications.insert(id, classify_row(&visibility_row, as_of_epoch));
        }
    }
    Ok(classifications)
}

pub fn classify_memory(
    conn: &Connection,
    memory_id: i64,
    as_of_epoch: i64,
) -> Result<MemoryVisibility> {
    Ok(classify_memories(conn, &[memory_id], as_of_epoch)?
        .remove(&memory_id)
        .unwrap_or_else(missing_row_visibility))
}

/// Shared G2 gate for every current-context reader.
///
/// SessionStart, UserPromptSubmit, `current_state`, and recall must call this
/// instead of treating `status='active'` as current.
/// Honors [`current_context_gate_mode`]; `classify_memory` keeps reporting the
/// unmodified classification for search, detail, inventory, and doctor.
pub fn admit_for_current_context(
    conn: &Connection,
    memory_id: i64,
    as_of_epoch: i64,
) -> Result<MemoryVisibility> {
    Ok(apply_gate_mode(classify_memory(
        conn,
        memory_id,
        as_of_epoch,
    )?))
}

/// Batched form of [`admit_for_current_context`] for readers that gate a whole
/// candidate set, so a context load costs a few statements instead of one per
/// memory.
pub fn admit_many_for_current_context(
    conn: &Connection,
    memory_ids: &[i64],
    as_of_epoch: i64,
) -> Result<HashMap<i64, MemoryVisibility>> {
    Ok(classify_memories(conn, memory_ids, as_of_epoch)?
        .into_iter()
        .map(|(id, visibility)| (id, apply_gate_mode(visibility)))
        .collect())
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

    /// Holds the shared process env lock while overriding the gate mode, so
    /// these tests cannot race other env-sensitive tests in the same binary.
    struct GateModeEnv {
        _lock: crate::runtime_config::TestEnvGuard,
        previous: Option<String>,
    }

    impl GateModeEnv {
        fn set(value: Option<&str>) -> Self {
            let lock = crate::runtime_config::ENV_LOCK
                .lock()
                .expect("env lock should acquire");
            let previous = std::env::var(CURRENT_CONTEXT_GATE_ENV).ok();
            apply(value);
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for GateModeEnv {
        fn drop(&mut self) {
            apply(self.previous.take().as_deref());
        }
    }

    fn apply(value: Option<&str>) {
        match value {
            Some(value) => unsafe { std::env::set_var(CURRENT_CONTEXT_GATE_ENV, value) },
            None => unsafe { std::env::remove_var(CURRENT_CONTEXT_GATE_ENV) },
        }
    }

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
    fn shadow_mode_admits_legacy_unverified_but_never_relaxes_lifecycle() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        // id=1 is excluded only for missing provenance; id=2 is quarantined.
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_trust_class)
             VALUES (1, '/repo', 'legacy', 'body', 'bugfix',
                     1, 1, 'active', 'local_tool_output')",
            [],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_trust_class)
             VALUES (2, '/repo', 'poisoned', 'body', 'bugfix',
                     1, 1, 'quarantined', 'user_prompt')",
            [],
        )?;

        let _guard = GateModeEnv::set(Some("shadow"));

        let legacy = admit_for_current_context(&conn, 1, 2)?;
        assert!(
            legacy.current_context_eligible,
            "shadow mode must admit legacy-unverified rows so injection can be measured"
        );
        assert_eq!(
            legacy.classification,
            MemoryVisibilityClass::LegacyUnverified,
            "shadow mode must preserve the real classification for reporting"
        );
        assert!(legacy.admitted_by_shadow_mode());

        let quarantined = admit_for_current_context(&conn, 2, 2)?;
        assert!(
            !quarantined.current_context_eligible,
            "shadow mode must not relax the quarantine security boundary"
        );
        assert!(!quarantined.admitted_by_shadow_mode());

        // classify_* reports the unmodified truth regardless of gate mode.
        assert!(!classify_memory(&conn, 1, 2)?.current_context_eligible);

        let batched = admit_many_for_current_context(&conn, &[1, 2], 2)?;
        assert!(batched[&1].current_context_eligible);
        assert!(!batched[&2].current_context_eligible);
        Ok(())
    }

    #[test]
    fn gate_mode_defaults_to_enforce_for_unset_and_unknown_values() {
        for value in [None, Some("yes-please-disable"), Some("off"), Some("")] {
            let _guard = GateModeEnv::set(value);
            assert_eq!(
                current_context_gate_mode(),
                CurrentContextGateMode::Enforce,
                "only the documented 'shadow' value may relax the gate, got {value:?}"
            );
        }
        let _shadow = GateModeEnv::set(Some(" shadow "));
        assert_eq!(
            current_context_gate_mode(),
            CurrentContextGateMode::Shadow,
            "surrounding whitespace should not defeat the documented value"
        );
    }

    #[test]
    fn batch_classification_matches_single_rows_and_marks_missing() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_trust_class)
             VALUES (1, '/repo', 'direct user', 'body', 'bugfix',
                     1, 1, 'active', 'user_prompt')",
            [],
        )?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, source_trust_class)
             VALUES (2, '/repo', 'legacy', 'body', 'bugfix',
                     1, 1, 'active', 'local_tool_output')",
            [],
        )?;

        let classifications = classify_memories(&conn, &[1, 2, 99, 1], 2)?;

        assert_eq!(classifications.len(), 3);
        assert_eq!(classifications[&1], classify_memory(&conn, 1, 2)?);
        assert!(classifications[&1].current_context_eligible);
        assert_eq!(
            classifications[&2],
            classify_memory(&conn, 2, 2)?,
            "batch and single-row paths must agree on excluded rows"
        );
        assert_eq!(
            classifications[&2].reason,
            MemoryVisibilityReason::ProvenanceMissing
        );
        assert_eq!(
            classifications[&99].reason,
            MemoryVisibilityReason::RowMissing
        );
        assert!(!classifications[&99].current_context_eligible);
        Ok(())
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
