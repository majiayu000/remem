use anyhow::Result;
use rusqlite::{params, Connection};

use super::{external_insert, outcome_candidate_id};
use crate::memory_candidate::route::{insert_external_candidate, ExternalCandidateOutcome};

#[test]
fn v076_schema_drift_is_reported_for_each_ledger_and_index() -> Result<()> {
    for (drop_sql, expected) in [
        (
            "DROP TABLE dream_quarantine_artifacts;",
            "dream_quarantine_artifacts",
        ),
        (
            "DROP INDEX idx_dream_quarantine_project_recent;",
            "idx_dream_quarantine_project_recent",
        ),
        (
            "DROP INDEX idx_dream_quarantine_candidate;",
            "idx_dream_quarantine_candidate",
        ),
        (
            "DROP TRIGGER dream_quarantine_artifacts_no_replace;",
            "dream_quarantine_artifacts_no_replace",
        ),
        (
            "DROP TRIGGER dream_quarantine_artifacts_initial_counters;",
            "dream_quarantine_artifacts_initial_counters",
        ),
        (
            "DROP TRIGGER dream_quarantine_artifacts_monotonic_recurrence;",
            "dream_quarantine_artifacts_monotonic_recurrence",
        ),
        (
            "DROP TRIGGER dream_quarantine_artifacts_no_delete;",
            "dream_quarantine_artifacts_no_delete",
        ),
        (
            "DROP TABLE external_candidate_identities;",
            "external_candidate_identities",
        ),
        (
            "DROP INDEX idx_external_candidate_identities_candidate;",
            "idx_external_candidate_identities_candidate",
        ),
        (
            "DROP TRIGGER external_candidate_identities_immutable_update;",
            "external_candidate_identities_immutable_update",
        ),
        (
            "DROP TRIGGER external_candidate_identities_monotonic_recurrence;",
            "external_candidate_identities_monotonic_recurrence",
        ),
        (
            "DROP TRIGGER external_candidate_identities_no_delete;",
            "external_candidate_identities_no_delete",
        ),
    ] {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute_batch(drop_sql)?;

        let drift = crate::migrate::validate_schema_invariants(&conn)?;
        assert!(
            drift.iter().any(|finding| finding.contains(expected)),
            "missing v076 object {expected} must be reported: {drift:?}"
        );
    }
    Ok(())
}

#[test]
fn v076_rejects_non_monotonic_identity_counts_and_null_recurrence_patterns() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let project = "/repos/v076-ledger-guards";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let candidate_id = outcome_candidate_id(insert_external_candidate(
        &conn,
        &external_insert(project_id, project),
    )?);
    let (identity_sha256, first_seen_epoch): (String, i64) = conn.query_row(
        "SELECT identity_sha256, first_seen_epoch FROM external_candidate_identities",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    conn.execute(
        "UPDATE external_candidate_identities
         SET last_seen_epoch = last_seen_epoch + 1,
             occurrence_count = occurrence_count + 1
         WHERE identity_sha256 = ?1",
        [&identity_sha256],
    )?;
    for sql in [
        "UPDATE external_candidate_identities
         SET occurrence_count = occurrence_count + 2",
        "UPDATE external_candidate_identities
         SET last_seen_epoch = last_seen_epoch - 1,
             occurrence_count = occurrence_count + 1",
        "UPDATE external_candidate_identities
         SET last_seen_epoch = last_seen_epoch",
    ] {
        assert!(
            conn.execute(sql, []).is_err(),
            "identity recurrence must advance exactly once without time regression"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT last_seen_epoch, occurrence_count
             FROM external_candidate_identities WHERE identity_sha256 = ?1",
            [&identity_sha256],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?,
        (first_seen_epoch + 1, 2)
    );

    for (kind, pattern_id, pattern_version) in [
        ("review_candidate", None, None),
        ("discarded_pattern", Some("pattern"), None),
        ("acknowledged_pattern", None, Some(1_i64)),
    ] {
        assert!(
            conn.execute(
                "INSERT INTO external_candidate_recurrences
                 (identity_sha256, canonical_candidate_id, candidate_id,
                  recurrence_kind, pattern_id, pattern_version, occurred_at_epoch)
                 VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)",
                params![
                    identity_sha256,
                    candidate_id,
                    kind,
                    pattern_id,
                    pattern_version,
                    first_seen_epoch,
                ],
            )
            .is_err(),
            "pattern-bearing recurrence kind {kind} must require a complete pattern pair"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM external_candidate_recurrences",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        0
    );
    assert!(matches!(
        insert_external_candidate(&conn, &external_insert(project_id, project))?,
        ExternalCandidateOutcome::Duplicate { candidate_id: retry_id } if retry_id == candidate_id
    ));
    Ok(())
}
