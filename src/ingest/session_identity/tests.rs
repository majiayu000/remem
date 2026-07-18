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
    resolve_fallback_group(&conn, "local", &fallback.fallback_session_id)
        .expect("resolve fallback");

    std::fs::write(
        &path,
        r#"{"type":"user","sessionId":"canonical-871","cwd":"/tmp/project","message":{"content":"first"}}"#,
    )
    .expect("promote fixture");
    let metadata = probe("local", root, &path, None).expect("probe metadata");
    let promoted_id = upsert_claim(&conn, &metadata, 2).expect("persist metadata");
    resolve_fallback_group(&conn, "local", &fallback.fallback_session_id)
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
    resolve_fallback_group(&conn, "local", &first.fallback_session_id).expect("resolve conflict");
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
    resolve_fallback_group(&conn, "local", &first.fallback_session_id).expect("retry resolution");
    assert_eq!(
        load(&conn, identity_id).expect("load sticky").status,
        "conflict"
    );
    std::fs::remove_file(path).expect("remove fixture");
}

#[test]
fn exact_collision_rewrites_every_persisted_evidence_reference() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    let path = temp_transcript(
        "evidence-rewrite",
        r#"{"type":"user","sessionId":"canonical-871","cwd":"/tmp/project","timestamp":100,"message":{"content":"same"}}"#,
    );
    let root = path.parent().context("fixture parent")?;
    let plan = probe("local", root, &path, None)?;
    let identity_id = upsert_claim(&conn, &plan, 1)?;
    resolve_fallback_group(&conn, "local", &plan.fallback_session_id)?;
    let identity = load(&conn, identity_id)?;
    let hash = crate::db::content_identity_hash(b"same");
    conn.execute(
        "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root, event_time_source
         ) VALUES (41, ?1, ?2, 'user', 'same', ?3, 'transcript',
                   999, 'local', 'legacy_unknown')",
        params![plan.fallback_session_id, plan.legacy_project, hash],
    )?;
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
         ) VALUES (9, 'raw_message:41:sha256', 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_lesson_feed_events (
            id, project, session_id, source, source_hash, lesson_memory_id,
            outcome_kind, status, evidence_raw_message_ids,
            created_at_epoch, updated_at_epoch
         ) VALUES (7, 'project', 'canonical-871', 'test', 'hash', 9,
                   'failure', 'saved', '[41,42]', 1, 1)",
        [],
    )?;

    let report = rekey_legacy_rows(&conn, &identity)?;

    assert_eq!(report.merged, 1);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_messages WHERE id = 41",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT evidence_raw_message_ids
             FROM memory_lesson_feed_events WHERE id = 7",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "[42]"
    );
    assert_eq!(
        conn.query_row(
            "SELECT source_evidence FROM memory_lessons WHERE memory_id = 9",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "raw_message:42:sha256"
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
    resolve_fallback_group(&conn, "local", &plan.fallback_session_id)?;
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
fn schema_aware_evidence_store_inventory_matches_rewrite_coverage() -> anyhow::Result<()> {
    let conn = setup_identity_db();
    let mut statement = conn.prepare(
        "SELECT m.name, p.name
         FROM sqlite_master m
         JOIN pragma_table_info(m.name) p
         WHERE m.type = 'table' AND p.name LIKE '%raw_message%'
         ORDER BY m.name, p.cid",
    )?;
    let named_raw_references = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    assert_eq!(
        named_raw_references,
        vec![(
            "memory_lesson_feed_events".to_string(),
            "evidence_raw_message_ids".to_string()
        )],
        "a new named raw-message reference store needs rewrite coverage"
    );
    let lesson_source_evidence: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('memory_lessons')
         WHERE name = 'source_evidence'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        lesson_source_evidence, 1,
        "generic lesson source evidence stores raw_message:<id>: tokens"
    );
    Ok(())
}
