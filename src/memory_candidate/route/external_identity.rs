use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::memory::poisoning::{scan_instruction_pattern, InstructionPatternMatch};

mod state;
use state::{
    existing_candidate_action, load_and_validate_candidate, validate_quarantine_match,
    ExistingCandidateAction, ExistingExternalCandidate,
};

#[derive(Debug, Clone)]
pub(crate) struct ExternalCandidateInsert<'a> {
    pub project_id: i64,
    pub source_project: &'a str,
    pub scope: &'a str,
    pub memory_type: &'a str,
    pub topic_key: &'a str,
    pub text: &'a str,
    pub confidence: f64,
    pub risk_class: &'a str,
    pub source_kind: &'a str,
    pub semantic_discriminator_sha256: Option<&'a str>,
    pub owner_scope: &'a str,
    pub owner_key: &'a str,
    pub target_project: Option<&'a str>,
    pub context_class: &'a str,
    pub routing_reason: &'a str,
    pub quarantine_match: Option<InstructionPatternMatch>,
}

impl ExternalCandidateInsert<'_> {
    fn identity(&self) -> ExternalCandidateIdentity<'_> {
        ExternalCandidateIdentity {
            source_kind: self.source_kind,
            memory_type: self.memory_type,
            semantic_discriminator_sha256: self.semantic_discriminator_sha256,
            source_project: self.source_project,
            owner_scope: self.owner_scope,
            owner_key: self.owner_key,
            target_project: self.target_project,
            topic_key: self.topic_key,
            text: self.text,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalCandidateIdentity<'a> {
    pub source_kind: &'a str,
    pub memory_type: &'a str,
    pub semantic_discriminator_sha256: Option<&'a str>,
    pub source_project: &'a str,
    pub owner_scope: &'a str,
    pub owner_key: &'a str,
    pub target_project: Option<&'a str>,
    pub topic_key: &'a str,
    pub text: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalCandidateOutcome {
    Inserted {
        candidate_id: i64,
        quarantined: bool,
    },
    Duplicate {
        candidate_id: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalCandidateDisposition {
    PendingReview,
    Quarantined,
    Duplicate,
}

#[derive(Debug)]
struct ExternalCandidateLedgerRow {
    candidate_id: i64,
    source_kind: String,
    memory_type: String,
    semantic_discriminator_sha256: Option<String>,
    source_project: String,
    owner_scope: String,
    owner_key: String,
    target_project: Option<String>,
    topic_key: String,
    text_sha256: String,
}

#[derive(Debug)]
struct ExternalCandidateDigests {
    identity_sha256: String,
    text_sha256: String,
}

#[derive(Debug)]
struct LegacyCandidateClaim {
    candidate_id: i64,
    first_seen_epoch: i64,
    occurrence_count: i64,
}

fn append_hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update([1]);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn append_optional_hash_field(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => append_hash_field(hasher, value),
        None => hasher.update([0]),
    }
}

fn external_candidate_digests(
    identity: &ExternalCandidateIdentity<'_>,
) -> ExternalCandidateDigests {
    let text_sha256 = format!("{:x}", Sha256::digest(identity.text.as_bytes()));
    let mut hasher = Sha256::new();
    append_hash_field(&mut hasher, "external-candidate-identity-v2");
    append_hash_field(&mut hasher, identity.source_kind);
    append_hash_field(&mut hasher, identity.memory_type);
    append_optional_hash_field(&mut hasher, identity.semantic_discriminator_sha256);
    append_hash_field(&mut hasher, identity.source_project);
    append_hash_field(&mut hasher, identity.owner_scope);
    append_hash_field(&mut hasher, identity.owner_key);
    append_optional_hash_field(&mut hasher, identity.target_project);
    append_hash_field(&mut hasher, identity.topic_key);
    append_hash_field(&mut hasher, identity.text);
    ExternalCandidateDigests {
        identity_sha256: format!("{:x}", hasher.finalize()),
        text_sha256,
    }
}

fn ledger_candidate_id(
    conn: &Connection,
    identity: &ExternalCandidateIdentity<'_>,
    digests: &ExternalCandidateDigests,
) -> Result<Option<i64>> {
    let row = conn
        .query_row(
            "SELECT candidate_id, source_kind, memory_type,
                    semantic_discriminator_sha256, source_project, owner_scope,
                    owner_key, target_project, topic_key, text_sha256
             FROM external_candidate_identities
             WHERE identity_sha256 = ?1",
            [&digests.identity_sha256],
            |row| {
                Ok(ExternalCandidateLedgerRow {
                    candidate_id: row.get(0)?,
                    source_kind: row.get(1)?,
                    memory_type: row.get(2)?,
                    semantic_discriminator_sha256: row.get(3)?,
                    source_project: row.get(4)?,
                    owner_scope: row.get(5)?,
                    owner_key: row.get(6)?,
                    target_project: row.get(7)?,
                    topic_key: row.get(8)?,
                    text_sha256: row.get(9)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.source_kind != identity.source_kind
        || row.memory_type != identity.memory_type
        || row.semantic_discriminator_sha256.as_deref() != identity.semantic_discriminator_sha256
        || row.source_project != identity.source_project
        || row.owner_scope != identity.owner_scope
        || row.owner_key != identity.owner_key
        || row.target_project.as_deref() != identity.target_project
        || row.topic_key != identity.topic_key
        || row.text_sha256 != digests.text_sha256
    {
        bail!(
            "external candidate identity hash collision for digest {}",
            digests.identity_sha256
        );
    }
    Ok(Some(row.candidate_id))
}

fn legacy_external_candidate_id(
    conn: &Connection,
    identity: &ExternalCandidateIdentity<'_>,
) -> Result<Option<i64>> {
    if identity.semantic_discriminator_sha256.is_some() {
        return Ok(None);
    }
    conn.query_row(
        "SELECT id FROM memory_candidates
         WHERE source_kind = ?1
           AND memory_type = ?2
           AND source_project = ?3
           AND owner_scope = ?4
           AND owner_key = ?5
           AND target_project IS ?6
           AND topic_key = ?7
           AND text = ?8
         ORDER BY CASE
             WHEN review_status = 'quarantined' THEN 0
             WHEN review_status = 'pending_review' THEN 1
             ELSE 2
         END, id ASC
         LIMIT 1",
        params![
            identity.source_kind,
            identity.memory_type,
            identity.source_project,
            identity.owner_scope,
            identity.owner_key,
            identity.target_project,
            identity.topic_key,
            identity.text,
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn legacy_external_candidate_claim(
    conn: &Connection,
    identity: &ExternalCandidateIdentity<'_>,
) -> Result<Option<LegacyCandidateClaim>> {
    if identity.semantic_discriminator_sha256.is_some() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, created_at_epoch FROM memory_candidates
         WHERE source_kind = ?1
           AND memory_type = ?2
           AND source_project = ?3
           AND owner_scope = ?4
           AND owner_key = ?5
           AND target_project IS ?6
           AND topic_key = ?7
           AND text = ?8
         ORDER BY CASE
             WHEN review_status = 'quarantined' THEN 0
             WHEN review_status = 'pending_review' THEN 1
             ELSE 2
         END, id ASC",
    )?;
    let rows = stmt.query_map(
        params![
            identity.source_kind,
            identity.memory_type,
            identity.source_project,
            identity.owner_scope,
            identity.owner_key,
            identity.target_project,
            identity.topic_key,
            identity.text,
        ],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let mut canonical_id = None;
    let mut first_seen_epoch = i64::MAX;
    let mut occurrence_count = 0;
    for row in rows {
        let (candidate_id, created_at_epoch) = row?;
        canonical_id.get_or_insert(candidate_id);
        first_seen_epoch = first_seen_epoch.min(created_at_epoch);
        occurrence_count += 1;
    }
    Ok(canonical_id.map(|candidate_id| LegacyCandidateClaim {
        candidate_id,
        first_seen_epoch,
        occurrence_count,
    }))
}

fn insert_external_identity(
    conn: &Connection,
    identity: &ExternalCandidateIdentity<'_>,
    digests: &ExternalCandidateDigests,
    candidate_id: i64,
    first_seen_epoch: i64,
    last_seen_epoch: i64,
    occurrence_count: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO external_candidate_identities
         (identity_sha256, candidate_id, source_kind, source_project,
          memory_type, semantic_discriminator_sha256, owner_scope, owner_key,
          target_project, topic_key, text_sha256, first_seen_epoch,
          last_seen_epoch, occurrence_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            digests.identity_sha256,
            candidate_id,
            identity.source_kind,
            identity.source_project,
            identity.memory_type,
            identity.semantic_discriminator_sha256,
            identity.owner_scope,
            identity.owner_key,
            identity.target_project,
            identity.topic_key,
            digests.text_sha256,
            first_seen_epoch,
            last_seen_epoch,
            occurrence_count,
        ],
    )?;
    Ok(())
}

fn validate_semantic_discriminator_sha256(value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| {
        value.len() != 64
            || value
                .bytes()
                .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    }) {
        bail!("external candidate semantic discriminator contract is invalid");
    }
    Ok(())
}

fn effective_candidate_id(
    conn: &Connection,
    identity_sha256: &str,
    canonical_candidate_id: i64,
) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT candidate_id FROM external_candidate_recurrences
             WHERE identity_sha256 = ?1 ORDER BY id DESC LIMIT 1",
            [identity_sha256],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(canonical_candidate_id))
}

fn record_recurrence(
    conn: &Connection,
    digests: &ExternalCandidateDigests,
    canonical_candidate_id: i64,
    candidate_id: i64,
    kind: &str,
    matched: Option<InstructionPatternMatch>,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO external_candidate_recurrences
         (identity_sha256, canonical_candidate_id, candidate_id,
          recurrence_kind, pattern_id, pattern_version, occurred_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            digests.identity_sha256,
            canonical_candidate_id,
            candidate_id,
            kind,
            matched.map(|value| value.pattern_id),
            matched.map(|value| value.pattern_set_version),
            now,
        ],
    )?;
    Ok(())
}

fn insert_candidate_row(
    conn: &Connection,
    insert: &ExternalCandidateInsert<'_>,
    quarantine_match: Option<InstructionPatternMatch>,
    now: i64,
) -> Result<i64> {
    let review_status = if quarantine_match.is_some() {
        "quarantined"
    } else {
        "pending_review"
    };
    let block_reason = if quarantine_match.is_some() {
        "quarantined_instruction_pattern"
    } else {
        "external_source_requires_review"
    };
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, routing_reason, context_class,
          source_kind, source_trust_class, auto_promote_block_reason,
          quarantine_pattern_id, quarantine_pattern_version)
         VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, ?7, ?8, ?9, ?9,
                 ?10, ?11, ?12, ?13, NULL, 0.5, ?14, ?15, ?16,
                 'external_content', ?17, ?18, ?19)",
        params![
            insert.project_id,
            insert.scope,
            insert.memory_type,
            insert.topic_key,
            insert.text,
            insert.confidence,
            insert.risk_class,
            review_status,
            now,
            insert.source_project,
            insert.target_project,
            insert.owner_scope,
            insert.owner_key,
            insert.routing_reason,
            insert.context_class,
            insert.source_kind,
            block_reason,
            quarantine_match.map(|matched| matched.pattern_id),
            quarantine_match.map(|matched| matched.pattern_set_version),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn handle_existing_candidate(
    conn: &Connection,
    insert: &ExternalCandidateInsert<'_>,
    digests: &ExternalCandidateDigests,
    canonical_candidate_id: i64,
    candidate: ExistingExternalCandidate,
    quarantine_match: Option<InstructionPatternMatch>,
    now: i64,
) -> Result<ExternalCandidateOutcome> {
    match existing_candidate_action(&candidate, quarantine_match) {
        ExistingCandidateAction::Duplicate => {}
        ExistingCandidateAction::QuarantinePending(matched) => {
            let updated = conn.execute(
                "UPDATE memory_candidates
                     SET review_status = 'quarantined',
                         auto_promote_block_reason = 'quarantined_instruction_pattern',
                         quarantine_pattern_id = ?2, quarantine_pattern_version = ?3,
                         updated_at_epoch = ?4
                 WHERE id = ?1 AND review_status = 'pending_review'",
                params![
                    candidate.id,
                    matched.pattern_id,
                    matched.pattern_set_version,
                    now
                ],
            )?;
            if updated != 1 {
                bail!("external candidate pending quarantine transition lost atomicity");
            }
        }
        ExistingCandidateAction::RefreshQuarantine(matched) => {
            let updated = conn.execute(
                "UPDATE memory_candidates
                 SET quarantine_pattern_id = ?2, quarantine_pattern_version = ?3,
                     updated_at_epoch = ?4
                 WHERE id = ?1 AND review_status = 'quarantined'",
                params![
                    candidate.id,
                    matched.pattern_id,
                    matched.pattern_set_version,
                    now
                ],
            )?;
            if updated != 1 {
                bail!("external candidate quarantine refresh lost atomicity");
            }
        }
        ExistingCandidateAction::RecordTerminalDuplicate => record_recurrence(
            conn,
            digests,
            canonical_candidate_id,
            candidate.id,
            "terminal_duplicate",
            None,
            now,
        )?,
        ExistingCandidateAction::RecordAcknowledgedPattern(matched) => record_recurrence(
            conn,
            digests,
            canonical_candidate_id,
            candidate.id,
            "acknowledged_pattern",
            Some(matched),
            now,
        )?,
        ExistingCandidateAction::RecordDiscardedPattern(matched) => record_recurrence(
            conn,
            digests,
            canonical_candidate_id,
            candidate.id,
            "discarded_pattern",
            Some(matched),
            now,
        )?,
        ExistingCandidateAction::CreateReviewCandidate(matched) => {
            let recurrence_candidate_id = insert_candidate_row(conn, insert, Some(matched), now)?;
            record_recurrence(
                conn,
                digests,
                canonical_candidate_id,
                recurrence_candidate_id,
                "review_candidate",
                Some(matched),
                now,
            )?;
            return Ok(ExternalCandidateOutcome::Inserted {
                candidate_id: recurrence_candidate_id,
                quarantined: true,
            });
        }
    }
    Ok(ExternalCandidateOutcome::Duplicate {
        candidate_id: candidate.id,
    })
}

#[cfg(test)]
pub(crate) fn external_candidate_exists(
    conn: &Connection,
    identity: &ExternalCandidateIdentity<'_>,
) -> Result<bool> {
    validate_semantic_discriminator_sha256(identity.semantic_discriminator_sha256)?;
    let digests = external_candidate_digests(identity);
    if ledger_candidate_id(conn, identity, &digests)?.is_some() {
        return Ok(true);
    }
    Ok(legacy_external_candidate_id(conn, identity)?.is_some())
}

pub(crate) fn external_candidate_disposition(
    conn: &Connection,
    identity: &ExternalCandidateIdentity<'_>,
    quarantine_match: Option<InstructionPatternMatch>,
) -> Result<ExternalCandidateDisposition> {
    validate_semantic_discriminator_sha256(identity.semantic_discriminator_sha256)?;
    validate_quarantine_match(quarantine_match)?;
    let digests = external_candidate_digests(identity);
    let candidate_id =
        if let Some(canonical_candidate_id) = ledger_candidate_id(conn, identity, &digests)? {
            Some(effective_candidate_id(
                conn,
                &digests.identity_sha256,
                canonical_candidate_id,
            )?)
        } else {
            legacy_external_candidate_id(conn, identity)?
        };
    let Some(candidate_id) = candidate_id else {
        return Ok(if quarantine_match.is_some() {
            ExternalCandidateDisposition::Quarantined
        } else {
            ExternalCandidateDisposition::PendingReview
        });
    };
    let candidate = load_and_validate_candidate(
        conn,
        candidate_id,
        identity.source_kind,
        identity.memory_type,
    )?;
    Ok(existing_candidate_action(&candidate, quarantine_match).disposition())
}

pub(crate) fn insert_external_candidate(
    conn: &Connection,
    insert: &ExternalCandidateInsert<'_>,
) -> Result<ExternalCandidateOutcome> {
    with_external_candidate_savepoint(conn, || insert_external_candidate_inner(conn, insert))
}

fn insert_external_candidate_inner(
    conn: &Connection,
    insert: &ExternalCandidateInsert<'_>,
) -> Result<ExternalCandidateOutcome> {
    let identity = insert.identity();
    validate_semantic_discriminator_sha256(identity.semantic_discriminator_sha256)?;
    let digests = external_candidate_digests(&identity);
    let now = chrono::Utc::now().timestamp();
    let quarantine_match = insert
        .quarantine_match
        .or_else(|| scan_instruction_pattern(insert.text));
    validate_quarantine_match(quarantine_match)?;

    let recurrence_updates = conn.execute(
        "UPDATE external_candidate_identities
         SET last_seen_epoch = MAX(last_seen_epoch, ?2),
             occurrence_count = occurrence_count + 1
         WHERE identity_sha256 = ?1",
        params![digests.identity_sha256, now],
    )?;
    if recurrence_updates > 1 {
        bail!("external candidate recurrence update lost identity uniqueness");
    }
    if recurrence_updates == 1 {
        let canonical_candidate_id = ledger_candidate_id(conn, &identity, &digests)?
            .ok_or_else(|| anyhow::anyhow!("external candidate recurrence lost ledger row"))?;
        let candidate_id =
            effective_candidate_id(conn, &digests.identity_sha256, canonical_candidate_id)?;
        let candidate = load_and_validate_candidate(
            conn,
            candidate_id,
            insert.source_kind,
            insert.memory_type,
        )?;
        return handle_existing_candidate(
            conn,
            insert,
            &digests,
            canonical_candidate_id,
            candidate,
            quarantine_match,
            now,
        );
    }

    if let Some(legacy) = legacy_external_candidate_claim(conn, &identity)? {
        if legacy.occurrence_count > 1 {
            crate::log::error(
                "external-candidate",
                &format!(
                    "multiple legacy candidates claimed: identity_sha256={} count={} canonical_candidate_id={}",
                    digests.identity_sha256, legacy.occurrence_count, legacy.candidate_id
                ),
            );
        }
        insert_external_identity(
            conn,
            &identity,
            &digests,
            legacy.candidate_id,
            legacy.first_seen_epoch,
            now.max(legacy.first_seen_epoch),
            legacy.occurrence_count + 1,
        )?;
        let candidate = load_and_validate_candidate(
            conn,
            legacy.candidate_id,
            insert.source_kind,
            insert.memory_type,
        )?;
        return handle_existing_candidate(
            conn,
            insert,
            &digests,
            legacy.candidate_id,
            candidate,
            quarantine_match,
            now,
        );
    }

    let candidate_id = insert_candidate_row(conn, insert, quarantine_match, now)?;
    insert_external_identity(conn, &identity, &digests, candidate_id, now, now, 1)?;
    Ok(ExternalCandidateOutcome::Inserted {
        candidate_id,
        quarantined: quarantine_match.is_some(),
    })
}

fn with_external_candidate_savepoint<T>(
    conn: &Connection,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_external_candidate_identity")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_external_candidate_identity")?;
            Ok(value)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_external_candidate_identity;
                 RELEASE SAVEPOINT remem_external_candidate_identity;",
            );
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!(
                    "external candidate identity rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}
