use super::*;

fn temp_transcript(name: &str, content: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "remem-gh871-{name}-{}-{}.jsonl",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&path, content).expect("write transcript fixture");
    path
}

fn setup_identity_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open fixture database");
    crate::migrate::run_migrations(&conn).expect("migrate fixture database");
    conn
}

#[test]
fn fallback_promotion_keeps_path_stable_identity() {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "promotion",
        r#"{"type":"user","cwd":"/tmp/project","message":{"content":"first"}}"#,
    );
    let root = path.parent().expect("fixture parent");
    let fallback = probe("local", root, &path, None).expect("probe fallback");
    let identity_id = upsert_claim(&conn, &fallback, 1).expect("persist fallback");
    resolve_fallback_group(
        &conn,
        fallback.host.map(InstallHost::as_db_value),
        "local",
        &fallback.fallback_session_id,
    )
    .expect("resolve fallback");

    std::fs::write(
        &path,
        r#"{"type":"user","sessionId":"canonical-871","cwd":"/tmp/project","message":{"content":"first"}}"#,
    )
    .expect("promote fixture");
    let metadata = probe("local", root, &path, None).expect("probe metadata");
    let promoted_id = upsert_claim(&conn, &metadata, 2).expect("persist metadata");
    resolve_fallback_group(
        &conn,
        metadata.host.map(InstallHost::as_db_value),
        "local",
        &fallback.fallback_session_id,
    )
    .expect("resolve metadata");
    let identity = load(&conn, identity_id).expect("load identity");

    assert_eq!(promoted_id, identity_id);
    assert_eq!(identity.canonical_session_id, "canonical-871");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_session_identity_claims
             WHERE transcript_identity_id = ?1",
            [identity_id],
            |row| row.get::<_, i64>(0)
        )
        .expect("count claims"),
        2
    );
    std::fs::remove_file(path).expect("remove fixture");
}

#[test]
fn conflicting_metadata_claims_are_sticky() {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "conflict",
        r#"{"type":"user","sessionId":"canonical-a","message":{"content":"first"}}"#,
    );
    let root = path.parent().expect("fixture parent");
    let first = probe("local", root, &path, None).expect("first probe");
    let identity_id = upsert_claim(&conn, &first, 1).expect("first claim");

    std::fs::write(
        &path,
        r#"{"type":"user","sessionId":"canonical-b","message":{"content":"first"}}"#,
    )
    .expect("rewrite fixture");
    let second = probe("local", root, &path, None).expect("second probe");
    upsert_claim(&conn, &second, 2).expect("second claim");
    resolve_fallback_group(
        &conn,
        first.host.map(InstallHost::as_db_value),
        "local",
        &first.fallback_session_id,
    )
    .expect("resolve conflict");
    assert_eq!(
        load(&conn, identity_id).expect("load conflict").status,
        "conflict"
    );

    std::fs::write(
        &path,
        r#"{"type":"user","sessionId":"canonical-a","message":{"content":"first"}}"#,
    )
    .expect("restore fixture");
    let retry = probe("local", root, &path, None).expect("retry probe");
    upsert_claim(&conn, &retry, 3).expect("retry claim");
    resolve_fallback_group(
        &conn,
        retry.host.map(InstallHost::as_db_value),
        "local",
        &first.fallback_session_id,
    )
    .expect("retry resolution");
    assert_eq!(
        load(&conn, identity_id).expect("load sticky").status,
        "conflict"
    );
    std::fs::remove_file(path).expect("remove fixture");
}

#[test]
fn established_host_survives_unknown_reprobe_and_rejects_conflict_before_mutation() {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "host-monotonic",
        r#"{"type":"user","sessionId":"stable","message":{"content":"first"}}"#,
    );
    let root = path.parent().expect("fixture parent");
    let mut stop_plan = probe("local", root, &path, None).expect("probe Stop transcript");
    stop_plan.host = Some(InstallHost::CodexCli);
    let identity_id = upsert_claim(&conn, &stop_plan, 1).expect("persist Stop host");

    let batch_plan = probe("local", root, &path, None).expect("probe unclassified batch path");
    assert_eq!(batch_plan.host, None);
    upsert_claim(&conn, &batch_plan, 2).expect("unknown reprobe preserves established host");
    assert_eq!(
        load(&conn, identity_id).expect("load preserved host").host,
        Some("codex-cli".to_string())
    );

    let mut conflicting_plan = batch_plan;
    conflicting_plan.host = Some(InstallHost::ClaudeCode);
    let error = upsert_claim(&conn, &conflicting_plan, 3)
        .expect_err("a different non-empty host must fail before mutation");
    assert!(error.to_string().contains("host provenance conflict"));
    let stored: (Option<String>, i64) = conn
        .query_row(
            "SELECT host, last_seen_at_epoch FROM raw_session_identities WHERE id = ?1",
            [identity_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("load host after rejected conflict");
    assert_eq!(stored, (Some("codex-cli".to_string()), 2));
    std::fs::remove_file(path).expect("remove fixture");
}

#[test]
fn same_fallback_id_resolves_independently_per_host() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    conn.execute_batch(
        "INSERT INTO raw_session_identities (
            id, source_root, transcript_path, host, fallback_session_id,
            canonical_session_id, project, legacy_project, status,
            observed_mtime_ns, observed_size_bytes,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES
            (71, 'local', '/tmp/.codex/sessions/shared.jsonl', 'codex-cli', 'shared',
             'shared', 'project', 'legacy', 'active', 1, 1, 1, 1),
            (72, 'local', '/tmp/.claude/projects/repo/shared.jsonl', 'claude-code', 'shared',
             'shared', 'project', 'legacy', 'active', 1, 1, 1, 1);
         INSERT INTO raw_session_identity_claims (
            transcript_identity_id, claimed_session_id, identity_source,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES
            (71, 'codex-canonical', 'transcript_metadata', 1, 1),
            (72, 'claude-canonical', 'transcript_metadata', 1, 1);",
    )?;

    resolve_fallback_group(&conn, Some("codex-cli"), "local", "shared")?;
    resolve_fallback_group(&conn, Some("claude-code"), "local", "shared")?;

    let rows = {
        let mut statement = conn.prepare(
            "SELECT host, status, canonical_session_id
             FROM raw_session_identities ORDER BY host",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    assert_eq!(
        rows,
        vec![
            (
                "claude-code".to_string(),
                "active".to_string(),
                "claude-canonical".to_string(),
            ),
            (
                "codex-cli".to_string(),
                "active".to_string(),
                "codex-canonical".to_string(),
            ),
        ]
    );
    Ok(())
}

#[test]
fn existing_group_conflict_is_inherited_by_later_identity() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    conn.execute_batch(
        "INSERT INTO raw_session_identities (
            id, source_root, transcript_path, fallback_session_id,
            canonical_session_id, project, legacy_project, status,
            conflict_reason, observed_mtime_ns, observed_size_bytes,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES
            (31, 'local', '/tmp/first/shared.jsonl', 'shared',
             'canonical-871', 'project', 'legacy', 'conflict',
             'stable_occurrence_mismatch', 1, 1, 1, 1),
            (32, 'local', '/tmp/second/shared.jsonl', 'shared',
             'canonical-871', 'project', 'legacy', 'active',
             NULL, 1, 1, 1, 1);
         INSERT INTO raw_session_identity_claims (
            transcript_identity_id, claimed_session_id, identity_source,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES
            (31, 'canonical-871', 'transcript_metadata', 1, 1),
            (32, 'canonical-871', 'transcript_metadata', 1, 1);",
    )?;

    resolve_fallback_group(&conn, None, "local", "shared")?;

    assert_eq!(
        conn.query_row(
            "SELECT GROUP_CONCAT(status || ':' || conflict_reason, ',')
             FROM (
                 SELECT status, conflict_reason
                 FROM raw_session_identities
                 WHERE source_root = 'local' AND fallback_session_id = 'shared'
                 ORDER BY id
             )",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "conflict:stable_occurrence_mismatch,conflict:stable_occurrence_mismatch"
    );
    Ok(())
}

#[test]
fn unresolved_legacy_rows_preserve_every_persisted_reference() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "evidence-rewrite",
        r#"{"type":"user","sessionId":"canonical-871","cwd":"/tmp/project","timestamp":100,"message":{"content":"same"}}"#,
    );
    let root = path.parent().context("fixture parent")?;
    let plan = probe("local", root, &path, None)?;
    let identity_id = upsert_claim(&conn, &plan, 1)?;
    resolve_fallback_group(
        &conn,
        plan.host.map(InstallHost::as_db_value),
        "local",
        &plan.fallback_session_id,
    )?;
    let identity = load(&conn, identity_id)?;
    let hash = crate::db::content_identity_hash(b"same");
    let insert_legacy = |id: i64, session_id: &str, project: &str| -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source
         ) VALUES (?1, ?2, ?3, 'user', 'same', ?4, 'transcript',
                   999, 'local', 'legacy_unknown')",
            params![id, session_id, project, hash],
        )?;
        Ok(())
    };
    insert_legacy(41, &plan.fallback_session_id, &plan.legacy_project)?;
    insert_legacy(43, &plan.canonical_session_id, &plan.legacy_project)?;
    insert_legacy(44, &plan.fallback_session_id, &plan.project)?;
    conn.execute(
        "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source,
            transcript_identity_id, transcript_record_ordinal
         ) VALUES (42, ?1, ?2, 'user', 'same', ?3, 'transcript',
                   100, 'local', 'transcript_event', ?4, 0)",
        params![plan.canonical_session_id, plan.project, hash, identity_id],
    )?;
    conn.execute(
        "INSERT INTO memories (
            id, project, title, content, memory_type,
            created_at_epoch, updated_at_epoch
         ) VALUES (9, 'project', 'lesson', 'body', 'lesson', 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_lessons (
            memory_id, source_evidence, last_reinforced_at_epoch
         ) VALUES (
            9,
            'raw_message:41:sha256 raw_message:43:sha256 raw_message:44:sha256',
            1
         )",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_lesson_feed_events (
            id, project, session_id, source, source_hash, lesson_memory_id,
            outcome_kind, status, evidence_raw_message_ids,
            created_at_epoch, updated_at_epoch
         ) VALUES (7, 'project', 'canonical-871', 'test', 'hash', 9,
                   'failure', 'saved', '[41,42,43,44]', 1, 1)",
        [],
    )?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    for (turn_id, raw_id, session_id, project) in [
        (
            701,
            41,
            plan.fallback_session_id.as_str(),
            plan.legacy_project.as_str(),
        ),
        (
            702,
            43,
            plan.canonical_session_id.as_str(),
            plan.legacy_project.as_str(),
        ),
        (
            703,
            44,
            plan.fallback_session_id.as_str(),
            plan.project.as_str(),
        ),
        (
            704,
            42,
            plan.canonical_session_id.as_str(),
            plan.project.as_str(),
        ),
    ] {
        conn.execute(
            "INSERT INTO session_turns (
                id, source_root, project, session_id, turn_index, user_message_id,
                result_status, started_at_epoch, capture_health, source_digest,
                projection_version, created_at_epoch, updated_at_epoch
             ) VALUES (?1, 'local', ?2, ?3, 1, ?4, 'unknown', 100,
                       'unavailable', 'stale', 1, 100, 100)",
            params![turn_id, project, session_id, raw_id],
        )?;
        conn.execute(
            "INSERT INTO session_turn_actions (
                session_turn_id, action_index, kind, summary, created_at_epoch
             ) VALUES (?1, 1, 'other', 'stale action', 100)",
            [turn_id],
        )?;
    }

    let error = rekey_legacy_rows(&conn, &identity)
        .expect_err("hostless legacy rows must fail before evidence mutation");

    assert!(error
        .downcast_ref::<crate::memory::raw_occurrence::RawIdentityConflict>()
        .is_some());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_messages WHERE id IN (41, 43, 44)",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        3
    );
    assert_eq!(
        conn.query_row(
            "SELECT evidence_raw_message_ids
             FROM memory_lesson_feed_events WHERE id = 7",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "[41,42,43,44]"
    );
    assert_eq!(
        conn.query_row(
            "SELECT source_evidence FROM memory_lessons WHERE memory_id = 9",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "raw_message:41:sha256 raw_message:43:sha256 raw_message:44:sha256"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM session_turns", [], |row| {
            row.get::<_, i64>(0)
        })?,
        4,
        "fail-closed rekey must preserve existing projections"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM session_turn_actions", [], |row| {
            row.get::<_, i64>(0)
        })?,
        4,
        "fail-closed rekey must preserve existing projection actions"
    );
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn ambiguous_or_inexact_collision_fails_before_any_mutation() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "collision-conflict",
        r#"{"type":"user","sessionId":"canonical-871","cwd":"/tmp/project","timestamp":100,"message":{"content":"old"}}"#,
    );
    let root = path.parent().context("fixture parent")?;
    let plan = probe("local", root, &path, None)?;
    let identity_id = upsert_claim(&conn, &plan, 1)?;
    resolve_fallback_group(
        &conn,
        plan.host.map(InstallHost::as_db_value),
        "local",
        &plan.fallback_session_id,
    )?;
    let identity = load(&conn, identity_id)?;
    let hash = crate::db::content_identity_hash(b"forced collision");
    conn.execute(
        "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source
         ) VALUES (51, ?1, ?2, 'user', 'old', ?3, 'transcript',
                   100, 'local', 'legacy_unknown')",
        params![plan.fallback_session_id, plan.legacy_project, hash],
    )?;
    conn.execute(
        "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source,
            transcript_identity_id, transcript_record_ordinal
         ) VALUES (52, ?1, ?2, 'user', 'different', ?3, 'transcript',
                   100, 'local', 'transcript_event', ?4, 0)",
        params![plan.canonical_session_id, plan.project, hash, identity_id],
    )?;

    let error = rekey_legacy_rows(&conn, &identity)
        .expect_err("same hash without exact stable equality must conflict");

    assert!(error
        .downcast_ref::<crate::memory::raw_occurrence::RawIdentityConflict>()
        .is_some());
    assert_eq!(
        conn.query_row(
            "SELECT GROUP_CONCAT(id || ':' || content, ',')
             FROM raw_messages WHERE id IN (51, 52) ORDER BY id",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "51:old,52:different"
    );
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn unmatched_legacy_aliases_fail_before_canonical_rekey() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "unmatched-aliases",
        r#"{"type":"user","sessionId":"canonical-871","cwd":"/tmp/project","timestamp":100,"message":{"content":"current"}}"#,
    );
    let root = path.parent().context("fixture parent")?;
    let plan = probe("local", root, &path, None)?;
    let identity_id = upsert_claim(&conn, &plan, 1)?;
    resolve_fallback_group(
        &conn,
        plan.host.map(InstallHost::as_db_value),
        "local",
        &plan.fallback_session_id,
    )?;
    let identity = load(&conn, identity_id)?;
    let hash = crate::db::content_identity_hash(b"removed legacy turn");
    for (id, session_id, project) in [
        (61, &plan.fallback_session_id, &plan.legacy_project),
        (62, &plan.canonical_session_id, &plan.project),
    ] {
        conn.execute(
            "INSERT INTO raw_messages (
                id, session_id, project, role, content, content_hash, source,
                created_at_epoch, source_root, event_time_source
             ) VALUES (?1, ?2, ?3, 'user', 'removed legacy turn', ?4,
                       'transcript', 100, 'local', 'legacy_unknown')",
            params![id, session_id, project, hash],
        )?;
    }

    let error = rekey_legacy_rows(&conn, &identity)
        .expect_err("hostless legacy aliases must remain unresolved");

    assert!(error
        .downcast_ref::<crate::memory::raw_occurrence::RawIdentityConflict>()
        .is_some());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_messages
             WHERE transcript_identity_id IS NULL AND content_hash = ?1",
            params![hash],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT GROUP_CONCAT(id || ':' || project || ':' || session_id, ',')
             FROM (SELECT id, project, session_id FROM raw_messages ORDER BY id)",
            [],
            |row| row.get::<_, String>(0)
        )?,
        format!(
            "61:{}:{},62:{}:{}",
            plan.legacy_project, plan.fallback_session_id, plan.project, plan.canonical_session_id
        )
    );
    std::fs::remove_file(path)?;
    Ok(())
}
