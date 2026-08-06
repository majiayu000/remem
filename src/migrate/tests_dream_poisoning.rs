use anyhow::Result;
use rusqlite::{params, Connection};

use super::run_migrations;
use crate::memory::poisoning::InstructionPatternMatch;
use crate::memory_candidate::route::{
    insert_external_candidate, ExternalCandidateInsert, ExternalCandidateOutcome,
};

mod concurrency;
mod schema;

fn external_insert<'a>(project_id: i64, project: &'a str) -> ExternalCandidateInsert<'a> {
    ExternalCandidateInsert {
        project_id,
        source_project: project,
        scope: "project",
        memory_type: "discovery",
        topic_key: "native-shared-topic",
        text: "The repository uses the shared topic content.",
        confidence: 0.5,
        risk_class: "high",
        source_kind: "claude_native",
        semantic_discriminator_sha256: None,
        owner_scope: "repo",
        owner_key: project,
        target_project: Some(project),
        context_class: "startup_core",
        routing_reason: "external native-memory regression fixture",
        quarantine_match: None,
    }
}

fn matched_external_insert<'a>(
    project_id: i64,
    project: &'a str,
    pattern_set_version: i64,
) -> ExternalCandidateInsert<'a> {
    ExternalCandidateInsert {
        quarantine_match: Some(pattern(
            "override_previous_instructions",
            pattern_set_version,
        )),
        ..external_insert(project_id, project)
    }
}

fn outcome_candidate_id(outcome: ExternalCandidateOutcome) -> i64 {
    match outcome {
        ExternalCandidateOutcome::Inserted { candidate_id, .. }
        | ExternalCandidateOutcome::Duplicate { candidate_id } => candidate_id,
    }
}

fn pattern(pattern_id: &'static str, pattern_set_version: i64) -> InstructionPatternMatch {
    InstructionPatternMatch {
        pattern_id,
        pattern_set_version,
    }
}

fn insert_artifact(
    conn: &Connection,
    candidate_id: i64,
    cluster: &str,
    decision_kind: &str,
    member_ids_json: &str,
    decision_ids_json: &str,
    intended_ids_json: &str,
    generated_topic_key: Option<&str>,
) -> rusqlite::Result<usize> {
    let payload_sha256 = "a".repeat(64);
    conn.execute(
        "INSERT INTO dream_quarantine_artifacts
         (project, cluster_signature, member_ids_json, source_candidate_id,
          decision_kind, decision_ids_json, decision_payload_sha256,
          intended_superseded_ids_json, generated_topic_key,
          generated_memory_type, generated_title, generated_content, generated_field,
          pattern_id, pattern_version, source_operation, source_trust_class,
          created_at_epoch, updated_at_epoch)
         VALUES ('/repo', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 CASE WHEN ?8 IS NULL THEN NULL ELSE 'decision' END,
                 CASE WHEN ?8 IS NULL THEN NULL ELSE 'generated title' END,
                 CASE WHEN ?8 IS NULL THEN NULL ELSE 'generated content' END,
                 CASE ?4
                     WHEN 'merge' THEN 'dream.content'
                     WHEN 'no_merge' THEN 'dream.no_merge_reason'
                     ELSE 'dream.conflict_reason'
                 END,
                 'override_previous_instructions', 1, 'dream',
                 'external_content', 1, 1)",
        params![
            cluster,
            member_ids_json,
            candidate_id,
            decision_kind,
            decision_ids_json,
            payload_sha256,
            intended_ids_json,
            generated_topic_key,
        ],
    )
}

#[test]
fn v076_creates_quarantine_and_external_identity_ledgers() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    assert_eq!(super::latest_schema_version(), 77);
    for index in [
        "idx_dream_quarantine_project_recent",
        "idx_dream_quarantine_candidate",
        "idx_dream_quarantine_backfill_memory",
        "idx_external_candidate_identities_candidate",
        "idx_external_candidate_recurrences_identity_recent",
        "idx_external_candidate_recurrences_candidate",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1
             )",
            [index],
            |row| row.get(0),
        )?;
        assert!(exists, "missing {index}");
    }

    conn.execute(
        "INSERT INTO memory_candidates
         (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_kind, source_trust_class)
         VALUES ('project', 'decision', 'generated-topic', 'generated content',
                 '[]', 0.5, 'high', 'quarantined', 1, 1,
                 'dream_model_output', 'external_content')",
        [],
    )?;
    let candidate_id = conn.last_insert_rowid();

    let identity_sha256 = "a".repeat(64);
    let text_sha256 = "b".repeat(64);
    conn.execute(
        "INSERT INTO external_candidate_identities
         (identity_sha256, candidate_id, source_kind, memory_type, source_project,
          owner_scope, owner_key, target_project, topic_key, text_sha256,
          first_seen_epoch, last_seen_epoch, occurrence_count)
         VALUES (?1, ?2, 'claude_memory_file', 'decision', '/repo', 'project', '/repo',
                 '/repo', 'topic', ?3, 1, 2, 1)",
        params![identity_sha256, candidate_id, text_sha256],
    )?;

    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(external_candidate_identities)")?
        .query_map([], |row| row.get(1))?
        .collect::<std::result::Result<_, _>>()?;
    assert!(!columns.iter().any(|column| column == "text"));
    assert!(
        columns
            .iter()
            .any(|column| column == "semantic_discriminator_sha256"),
        "semantic identity binding must be persisted in the immutable ledger"
    );
    assert_eq!(
        conn.query_row(
            "SELECT occurrence_count FROM external_candidate_identities
             WHERE identity_sha256 = ?1",
            [&identity_sha256],
            |row| row.get::<_, i64>(0),
        )?,
        1
    );

    conn.execute_batch("PRAGMA recursive_triggers=OFF;")?;
    let replace_error = conn
        .execute(
            "INSERT OR REPLACE INTO external_candidate_identities
             (identity_sha256, candidate_id, source_kind, memory_type, source_project,
              owner_scope, owner_key, target_project, topic_key, text_sha256,
              first_seen_epoch, last_seen_epoch, occurrence_count)
             VALUES (?1, ?2, 'replacement', 'decision', '/wrong', 'project', '/wrong',
                     '/wrong', 'wrong', ?3, 1, 1, 1)",
            params![identity_sha256, candidate_id, "c".repeat(64)],
        )
        .expect_err("BEFORE INSERT guard must block OR REPLACE");
    assert!(
        replace_error
            .to_string()
            .contains("external candidate identity already exists"),
        "{replace_error}"
    );
    assert_eq!(
        conn.query_row(
            "SELECT source_kind FROM external_candidate_identities
             WHERE identity_sha256 = ?1",
            [&identity_sha256],
            |row| row.get::<_, String>(0),
        )?,
        "claude_memory_file"
    );

    for (identity, text_hash, first_seen, last_seen, count) in [
        ("short".to_string(), "c".repeat(64), 1, 1, 1),
        ("c".repeat(64), "SHORT".to_string(), 1, 1, 1),
        ("d".repeat(64), "e".repeat(64), 1, 1, 0),
        ("e".repeat(64), "f".repeat(64), 2, 1, 1),
    ] {
        assert!(
            conn.execute(
                "INSERT INTO external_candidate_identities
                 (identity_sha256, candidate_id, source_kind, memory_type, source_project,
                  owner_scope, owner_key, target_project, topic_key, text_sha256,
                  first_seen_epoch, last_seen_epoch, occurrence_count)
                 VALUES (?1, ?2, 'claude_memory_file', 'decision', '/repo', 'project', '/repo',
                         '/repo', 'topic', ?3, ?4, ?5, ?6)",
                params![
                    identity,
                    candidate_id,
                    text_hash,
                    first_seen,
                    last_seen,
                    count
                ],
            )
            .is_err(),
            "invalid ledger hash/count/timestamp must fail closed"
        );
    }
    for (identity, discriminator) in [
        ("f".repeat(64), "short".to_string()),
        ("0".repeat(64), "A".repeat(64)),
    ] {
        assert!(
            conn.execute(
                "INSERT INTO external_candidate_identities
                 (identity_sha256, candidate_id, source_kind, memory_type,
                  semantic_discriminator_sha256, source_project, owner_scope, owner_key,
                  target_project, topic_key, text_sha256, first_seen_epoch,
                  last_seen_epoch, occurrence_count)
                 VALUES (?1, ?2, 'dream_model_output', 'decision', ?3, '/repo',
                         'project', '/repo', '/repo', 'topic', ?4, 1, 1, 1)",
                params![identity, candidate_id, discriminator, "1".repeat(64)],
            )
            .is_err(),
            "invalid semantic discriminator must fail closed at the ledger boundary"
        );
    }
    assert!(
        conn.execute(
            "INSERT INTO external_candidate_identities
             (identity_sha256, candidate_id, source_kind, memory_type, source_project,
              owner_scope, owner_key, target_project, topic_key, text_sha256,
              first_seen_epoch, last_seen_epoch, occurrence_count)
             VALUES (?1, ?2, 'claude_memory_file', 'decision', '/repo', 'project', '/repo',
                     '/repo', 'other-topic', ?3, 1, 1, 1)",
            params![identity_sha256, candidate_id, "f".repeat(64)],
        )
        .is_err(),
        "identity digest must be globally unique"
    );
    assert!(
        conn.execute(
            "DELETE FROM memory_candidates WHERE id = ?1",
            [candidate_id],
        )
        .is_err(),
        "ledger provenance must prevent candidate deletion"
    );
    assert!(
        conn.execute(
            "UPDATE external_candidate_identities SET memory_type = 'rewritten'",
            [],
        )
        .is_err(),
        "identity fields must be immutable"
    );
    assert!(
        conn.execute(
            "UPDATE external_candidate_identities
             SET semantic_discriminator_sha256 = ?1",
            ["2".repeat(64)],
        )
        .is_err(),
        "semantic discriminator must be immutable"
    );
    assert!(
        conn.execute("DELETE FROM external_candidate_identities", [])
            .is_err(),
        "identity ledger rows must be immutable"
    );
    insert_artifact(
        &conn,
        candidate_id,
        "cluster-a",
        "merge",
        "[1,2]",
        "[1]",
        "[1]",
        Some("generated-topic"),
    )?;

    assert!(
        insert_artifact(
            &conn,
            candidate_id,
            "cluster-b",
            "no_merge",
            "[]",
            "[]",
            "[]",
            None,
        )
        .is_err(),
        "empty source-memory provenance must fail closed"
    );
    assert!(
        insert_artifact(
            &conn,
            999999,
            "cluster-c",
            "merge",
            "[1,2]",
            "[1]",
            "[1]",
            Some("generated-topic"),
        )
        .is_err(),
        "artifact provenance must reference a real review candidate"
    );

    for (cluster, decision, members, decision_ids, intended, generated_topic) in [
        (
            "unsorted",
            "merge",
            "[1,2]",
            "[2,1]",
            "[2,1]",
            Some("topic"),
        ),
        (
            "duplicate",
            "merge",
            "[1,2]",
            "[1,1]",
            "[1,1]",
            Some("topic"),
        ),
        (
            "non-positive",
            "merge",
            "[0,1]",
            "[0]",
            "[0]",
            Some("topic"),
        ),
        (
            "member-unsorted",
            "merge",
            "[2,1]",
            "[1]",
            "[1]",
            Some("topic"),
        ),
        (
            "member-duplicate",
            "merge",
            "[1,1]",
            "[1]",
            "[1]",
            Some("topic"),
        ),
        (
            "member-non-positive",
            "merge",
            "[0,1]",
            "[1]",
            "[1]",
            Some("topic"),
        ),
        ("empty-merge", "merge", "[1,2]", "[]", "[]", Some("topic")),
        ("not-subset", "merge", "[1,2]", "[3]", "[3]", Some("topic")),
        (
            "merge-mismatch",
            "merge",
            "[1,2]",
            "[2]",
            "[1]",
            Some("topic"),
        ),
        ("conflict-short", "conflict", "[1,2]", "[1]", "[]", None),
        (
            "conflict-unsorted",
            "conflict",
            "[1,2]",
            "[2,1]",
            "[]",
            None,
        ),
        (
            "conflict-not-subset",
            "conflict",
            "[1,2]",
            "[1,3]",
            "[]",
            None,
        ),
        ("no-merge-ids", "no_merge", "[1,2]", "[1]", "[]", None),
        (
            "reason-has-intended",
            "conflict",
            "[1,2]",
            "[1,2]",
            "[1]",
            None,
        ),
        ("merge-no-fields", "merge", "[1,2]", "[1]", "[1]", None),
        (
            "merge-blank-topic",
            "merge",
            "[1,2]",
            "[1]",
            "[1]",
            Some(" "),
        ),
        (
            "reason-has-fields",
            "conflict",
            "[1,2]",
            "[1,2]",
            "[]",
            Some("topic"),
        ),
    ] {
        assert!(
            insert_artifact(
                &conn,
                candidate_id,
                cluster,
                decision,
                members,
                decision_ids,
                intended,
                generated_topic,
            )
            .is_err(),
            "invalid decision provenance must fail for {cluster}"
        );
    }
    conn.execute(
        "UPDATE dream_quarantine_artifacts
         SET version = version + 1, occurrence_count = occurrence_count + 1,
             updated_at_epoch = 2 WHERE cluster_signature = 'cluster-a'",
        [],
    )?;
    for sql in [
        "UPDATE dream_quarantine_artifacts SET version = version + 2,
             occurrence_count = occurrence_count + 1, updated_at_epoch = 3",
        "UPDATE dream_quarantine_artifacts SET version = version + 1,
             occurrence_count = occurrence_count + 2, updated_at_epoch = 3",
        "UPDATE dream_quarantine_artifacts SET version = version + 1,
             occurrence_count = occurrence_count + 1, updated_at_epoch = 0",
    ] {
        assert!(
            conn.execute(sql, []).is_err(),
            "non-monotonic update must fail"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT version, occurrence_count, updated_at_epoch
             FROM dream_quarantine_artifacts WHERE cluster_signature = 'cluster-a'",
            [],
            |row| Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?
            )),
        )?,
        (2, 2, 2),
        "failed recurrence updates must remain atomic"
    );
    assert!(
        conn.execute(
            "INSERT INTO dream_quarantine_artifacts
             SELECT NULL, 7, project, 'bad-initial-counters', member_ids_json,
                    source_candidate_id, decision_kind, decision_ids_json,
                    decision_payload_sha256, intended_superseded_ids_json,
                    generated_topic_key, generated_memory_type, generated_title,
                    generated_content, generated_field, pattern_id, pattern_version,
                    source_operation, source_trust_class, 7, created_at_epoch,
                    updated_at_epoch
             FROM dream_quarantine_artifacts WHERE cluster_signature = 'cluster-a'",
            [],
        )
        .is_err(),
        "artifact counters must start at one"
    );
    conn.execute_batch("PRAGMA recursive_triggers=OFF;")?;
    assert!(
        conn.execute(
            "INSERT OR REPLACE INTO dream_quarantine_artifacts
             SELECT * FROM dream_quarantine_artifacts
             WHERE cluster_signature = 'cluster-a'",
            [],
        )
        .is_err(),
        "artifact replacement must fail with recursive triggers disabled"
    );
    assert!(
        conn.execute(
            "DELETE FROM dream_quarantine_artifacts
             WHERE cluster_signature = 'cluster-a'",
            [],
        )
        .is_err(),
        "artifact audit rows must be append-only"
    );
    assert!(
        conn.execute(
            "UPDATE dream_quarantine_artifacts
             SET intended_superseded_ids_json = '[2]'
             WHERE cluster_signature = 'cluster-a'",
            [],
        )
        .is_err(),
        "artifact payload must be immutable"
    );

    conn.execute(
        "INSERT INTO memory_candidates
         (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch,
          source_kind, source_trust_class)
         VALUES ('project', 'decision', 'other-topic', 'other content', '[]',
                 0.5, 'high', 'quarantined', 2, 2,
                 'dream_model_output', 'external_content')",
        [],
    )?;
    let second_candidate_id = conn.last_insert_rowid();
    insert_artifact(
        &conn,
        second_candidate_id,
        "cluster-a",
        "conflict",
        "[1,2]",
        "[1,2]",
        "[]",
        None,
    )?;
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM dream_quarantine_artifacts
             WHERE cluster_signature = 'cluster-a'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        2,
        "new candidate provenance must append instead of rebinding the old artifact"
    );
    Ok(())
}

#[test]
fn quarantine_recurrence_respects_review_acknowledgement_and_rejection() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    let project = "/repos/quarantine-recurrence";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let canonical_id = outcome_candidate_id(insert_external_candidate(
        &conn,
        &external_insert(project_id, project),
    )?);

    let v1 = matched_external_insert(project_id, project, 1);
    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &v1)?),
        canonical_id
    );
    let state: (String, String, i64, i64) = conn.query_row(
        "SELECT review_status, quarantine_pattern_id, quarantine_pattern_version, version
         FROM memory_candidates WHERE id = ?1",
        [canonical_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        state,
        (
            "quarantined".into(),
            "override_previous_instructions".into(),
            1,
            2,
        )
    );

    let v2 = matched_external_insert(project_id, project, 2);
    insert_external_candidate(&conn, &v2)?;
    assert_eq!(
        conn.query_row(
            "SELECT version FROM memory_candidates WHERE id = ?1",
            [canonical_id],
            |row| row.get::<_, i64>(0),
        )?,
        3,
        "quarantine refresh must increment the candidate version exactly once"
    );
    conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'approved',
             acknowledged_pattern_id = quarantine_pattern_id,
             acknowledged_pattern_version = quarantine_pattern_version
         WHERE id = ?1",
        [canonical_id],
    )?;
    assert!(matches!(
        insert_external_candidate(&conn, &v2)?,
        ExternalCandidateOutcome::Duplicate { candidate_id } if candidate_id == canonical_id
    ));

    let v3 = matched_external_insert(project_id, project, 3);
    let recurrence_id = match insert_external_candidate(&conn, &v3)? {
        ExternalCandidateOutcome::Inserted {
            candidate_id,
            quarantined: true,
        } => candidate_id,
        outcome => anyhow::bail!("changed unacknowledged pattern must reopen review: {outcome:?}"),
    };
    assert_ne!(recurrence_id, canonical_id);
    assert_eq!(
        conn.query_row(
            "SELECT candidate_id FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        canonical_id,
        "immutable ledger canonical must not be rebound"
    );
    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &v3)?),
        recurrence_id,
        "an open recurrence candidate must absorb retries"
    );

    conn.execute(
        "UPDATE memory_candidates SET review_status = 'discarded' WHERE id = ?1",
        [recurrence_id],
    )?;
    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &v3)?),
        recurrence_id
    );
    assert_eq!(
        conn.query_row(
            "SELECT recurrence_kind FROM external_candidate_recurrences
             ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )?,
        "discarded_pattern"
    );

    let v4 = matched_external_insert(project_id, project, 4);
    let newest_id = outcome_candidate_id(insert_external_candidate(&conn, &v4)?);
    assert_ne!(newest_id, recurrence_id);
    conn.execute(
        "UPDATE memory_candidates SET source_trust_class = 'local_tool_output'
         WHERE id = ?1",
        [newest_id],
    )?;
    let count_before: i64 = conn.query_row(
        "SELECT occurrence_count FROM external_candidate_identities",
        [],
        |row| row.get(0),
    )?;
    assert!(insert_external_candidate(&conn, &v4).is_err());
    assert_eq!(
        conn.query_row(
            "SELECT occurrence_count FROM external_candidate_identities",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        count_before,
        "source contract validation must precede recurrence mutation"
    );
    Ok(())
}

#[test]
fn legacy_duplicates_are_preserved_and_claimed_deterministically() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    let project = "/repos/legacy";
    let project_id = crate::db::capture::ensure_project_row(&conn, project)?;
    let insert = external_insert(project_id, project);
    for (status, created_at) in [
        ("approved", 10),
        ("pending_review", 20),
        ("quarantined", 30),
    ] {
        conn.execute(
            "INSERT INTO memory_candidates
             (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
              source_project, target_project, owner_scope, owner_key, source_kind,
              source_trust_class)
             VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, ?7, ?8, ?9, ?9,
                     ?10, ?11, ?12, ?13, ?14, 'external_content')",
            params![
                insert.project_id,
                insert.scope,
                insert.memory_type,
                insert.topic_key,
                insert.text,
                insert.confidence,
                insert.risk_class,
                status,
                created_at,
                insert.source_project,
                insert.target_project,
                insert.owner_scope,
                insert.owner_key,
                insert.source_kind,
            ],
        )?;
        if status == "quarantined" {
            conn.execute(
                "UPDATE memory_candidates
                 SET quarantine_pattern_id = 'authority_claim',
                     quarantine_pattern_version = 1
                 WHERE id = ?1",
                [conn.last_insert_rowid()],
            )?;
        }
    }
    let actionable_id: i64 = conn.query_row(
        "SELECT id FROM memory_candidates WHERE review_status = 'quarantined'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(
        outcome_candidate_id(insert_external_candidate(&conn, &insert)?),
        actionable_id
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| row
            .get::<_, i64>(0))?,
        3,
        "legacy rows must never be deleted or merged"
    );
    let claim: (i64, i64, i64) = conn.query_row(
        "SELECT candidate_id, first_seen_epoch, occurrence_count
         FROM external_candidate_identities",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(claim, (actionable_id, 10, 4));
    Ok(())
}
