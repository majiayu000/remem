use anyhow::Result;
use rusqlite::{params, Connection};

use super::{external_insert, outcome_candidate_id, pattern};
use crate::memory_candidate::route::{
    external_candidate_exists, insert_external_candidate, ExternalCandidateIdentity,
    ExternalCandidateInsert, ExternalCandidateOutcome,
};

const SEMANTIC_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SEMANTIC_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn external_identity_retry_is_read_only_on_probe_and_counts_recurrence() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    super::run_migrations(&conn)?;
    let project = "/repos/retry";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let insert = external_insert(project_id, project);
    let first_id = outcome_candidate_id(insert_external_candidate(&conn, &insert)?);
    let identity = ExternalCandidateIdentity {
        source_kind: insert.source_kind,
        memory_type: insert.memory_type,
        semantic_discriminator_sha256: insert.semantic_discriminator_sha256,
        source_project: insert.source_project,
        owner_scope: insert.owner_scope,
        owner_key: insert.owner_key,
        target_project: insert.target_project,
        topic_key: insert.topic_key,
        text: insert.text,
    };
    assert!(external_candidate_exists(&conn, &identity)?);
    assert_eq!(
        conn.query_row(
            "SELECT occurrence_count FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1,
        "read-only existence probes must not mutate recurrence state"
    );

    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &insert)?),
        first_id
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT occurrence_count FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        2
    );

    let tool_insert = ExternalCandidateInsert {
        owner_scope: "tool",
        owner_key: "claude-code",
        target_project: None,
        context_class: "search_only",
        ..external_insert(project_id, project)
    };
    let tool_id = outcome_candidate_id(insert_external_candidate(&conn, &tool_insert)?);
    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &tool_insert)?),
        tool_id,
        "NULL target_project must participate deterministically in identity"
    );
    Ok(())
}

#[test]
fn terminal_and_edited_candidates_remain_closed_on_recurrence() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    super::run_migrations(&conn)?;
    let project = "/repos/terminal";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let insert = external_insert(project_id, project);
    let candidate_id = outcome_candidate_id(insert_external_candidate(&conn, &insert)?);

    for (index, status) in ["approved", "discarded", "noop"].into_iter().enumerate() {
        conn.execute(
            "UPDATE memory_candidates SET review_status = ?2 WHERE id = ?1",
            params![candidate_id, status],
        )?;
        assert_eq!(
            outcome_candidate_id(insert_external_candidate(&conn, &insert)?),
            candidate_id
        );
        assert_eq!(
            conn.query_row(
                "SELECT review_status FROM memory_candidates WHERE id = ?1",
                [candidate_id],
                |row| row.get::<_, String>(0),
            )?,
            status
        );
        assert_eq!(
            conn.query_row(
                "SELECT occurrence_count FROM external_candidate_identities",
                [],
                |row| row.get::<_, i64>(0),
            )?,
            index as i64 + 2
        );
    }

    conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'edited', text = 'reviewer-authored replacement',
             memory_type = 'preference'
         WHERE id = ?1",
        [candidate_id],
    )?;
    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &insert)?),
        candidate_id,
        "the immutable ledger must retain the original external identity after an edit"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    Ok(())
}

#[test]
fn memory_type_is_part_of_external_candidate_identity() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    super::run_migrations(&conn)?;
    let project = "/repos/memory-type-identity";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let discovery = external_insert(project_id, project);
    let preference = ExternalCandidateInsert {
        memory_type: "preference",
        ..external_insert(project_id, project)
    };
    let discovery_id = outcome_candidate_id(insert_external_candidate(&conn, &discovery)?);
    let preference_id = outcome_candidate_id(insert_external_candidate(&conn, &preference)?);
    assert_ne!(discovery_id, preference_id);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        2,
        "same route/topic/text with a different memory_type must not collide or legacy-claim"
    );
    Ok(())
}

#[test]
fn semantic_discriminator_skips_unbound_legacy_candidate_and_rejects_bad_digest() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    super::run_migrations(&conn)?;
    let project = "/repos/dream-semantic-legacy";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let insert = ExternalCandidateInsert {
        memory_type: "decision",
        source_kind: "dream_model_output",
        semantic_discriminator_sha256: Some(SEMANTIC_A),
        quarantine_match: Some(pattern("override_previous_instructions", 1)),
        ..external_insert(project_id, project)
    };
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key, source_kind,
          source_trust_class, auto_promote_block_reason, quarantine_pattern_id,
          quarantine_pattern_version)
         VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, ?7, 'quarantined', 1, 1,
                 ?8, ?9, ?10, ?11, ?12, 'external_content',
                 'quarantined_instruction_pattern', ?13, ?14)",
        params![
            insert.project_id,
            insert.scope,
            insert.memory_type,
            insert.topic_key,
            insert.text,
            insert.confidence,
            insert.risk_class,
            insert.source_project,
            insert.target_project,
            insert.owner_scope,
            insert.owner_key,
            insert.source_kind,
            insert.quarantine_match.map(|matched| matched.pattern_id),
            insert
                .quarantine_match
                .map(|matched| matched.pattern_set_version),
        ],
    )?;
    let unbound_legacy_id = conn.last_insert_rowid();

    let candidate_id = outcome_candidate_id(insert_external_candidate(&conn, &insert)?);
    assert_ne!(candidate_id, unbound_legacy_id);
    let ledger: (i64, Option<String>) = conn.query_row(
        "SELECT candidate_id, semantic_discriminator_sha256
         FROM external_candidate_identities",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(ledger, (candidate_id, Some(SEMANTIC_A.to_string())));

    let candidate_count_before: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let invalid = ExternalCandidateInsert {
        semantic_discriminator_sha256: Some("not-a-lowercase-sha256"),
        ..insert
    };
    let error = insert_external_candidate(&conn, &invalid).unwrap_err();
    assert!(error
        .to_string()
        .contains("semantic discriminator contract"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get::<_, i64>(0)
        })?,
        candidate_count_before,
        "invalid semantic identity must not leave a partial candidate"
    );
    Ok(())
}

#[test]
fn dream_semantic_supersede_reopens_a_to_b_to_a_without_reopening_human_discard() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    super::run_migrations(&conn)?;
    let project = "/repos/dream-semantic-aba";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let a = ExternalCandidateInsert {
        memory_type: "decision",
        source_kind: "dream_model_output",
        semantic_discriminator_sha256: Some(SEMANTIC_A),
        quarantine_match: Some(pattern("override_previous_instructions", 1)),
        ..external_insert(project_id, project)
    };
    let b = ExternalCandidateInsert {
        semantic_discriminator_sha256: Some(SEMANTIC_B),
        ..a.clone()
    };

    let a_id = outcome_candidate_id(insert_external_candidate(&conn, &a)?);
    let b_id = outcome_candidate_id(insert_external_candidate(&conn, &b)?);
    assert_ne!(
        a_id, b_id,
        "different semantic decisions need distinct review state"
    );
    conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'discarded', review_actor = 'system:dream',
             review_action_source = 'dream_semantic_superseded'
         WHERE id = ?1",
        [a_id],
    )?;

    let reopened_a_id = match insert_external_candidate(&conn, &a)? {
        ExternalCandidateOutcome::Inserted {
            candidate_id,
            quarantined: true,
        } => candidate_id,
        outcome => anyhow::bail!("superseded A must reopen after A→B→A: {outcome:?}"),
    };
    assert_ne!(reopened_a_id, a_id);
    assert_ne!(reopened_a_id, b_id);
    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &a)?),
        reopened_a_id,
        "the reopened A candidate must absorb an exact retry"
    );
    let recurrence: (i64, i64, String) = conn.query_row(
        "SELECT canonical_candidate_id, candidate_id, recurrence_kind
         FROM external_candidate_recurrences ORDER BY id DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(recurrence, (a_id, reopened_a_id, "review_candidate".into()));
    Ok(())
}

#[test]
fn collision_and_ledger_failure_fail_closed_without_partial_candidate() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    super::run_migrations(&conn)?;
    let project = "/repos/fail-closed";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let insert = external_insert(project_id, project);
    insert_external_candidate(&conn, &insert)?;
    conn.execute_batch("DROP TRIGGER external_candidate_identities_immutable_update;")?;
    conn.execute(
        "UPDATE external_candidate_identities SET source_kind = 'corrupt-source'",
        [],
    )?;
    let error = insert_external_candidate(&conn, &insert).unwrap_err();
    assert!(error.to_string().contains("hash collision"));
    assert_eq!(
        conn.query_row(
            "SELECT occurrence_count FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        1,
        "collision validation must happen before recurrence mutation"
    );

    let isolated = Connection::open_in_memory()?;
    super::run_migrations(&isolated)?;
    let project_id = crate::db::capture::ensure_project_row(&isolated, project)?;
    isolated.execute_batch(
        "CREATE TRIGGER reject_external_identity
         BEFORE INSERT ON external_candidate_identities
         BEGIN SELECT RAISE(FAIL, 'ledger unavailable'); END;",
    )?;
    assert!(insert_external_candidate(&isolated, &external_insert(project_id, project)).is_err());
    assert_eq!(
        isolated.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| row
            .get::<_, i64>(0))?,
        0,
        "candidate insert must roll back when immutable ledger persistence fails"
    );
    Ok(())
}

#[test]
fn concurrent_wal_imports_claim_one_identity_atomically() -> Result<()> {
    let path = crate::db::test_support::unique_temp_db_path("external-identity-race");
    let conn = Connection::open(&path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    super::run_migrations(&conn)?;
    let project = "/repos/wal-race";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        handles.push(std::thread::spawn(
            move || -> Result<ExternalCandidateOutcome> {
                let conn = Connection::open(path)?;
                conn.busy_timeout(std::time::Duration::from_secs(30))?;
                conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                barrier.wait();
                insert_external_candidate(&conn, &external_insert(project_id, project))
            },
        ));
    }
    let outcomes = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("external identity thread panicked"))?
        })
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        outcome_candidate_id(outcomes[0]),
        outcome_candidate_id(outcomes[1])
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| row
            .get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT occurrence_count FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        2
    );
    drop(conn);
    crate::db::test_support::cleanup_temp_db_files(&path);
    Ok(())
}
