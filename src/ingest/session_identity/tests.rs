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
