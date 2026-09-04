use super::*;
use crate::memory::raw_query::{query_raw_session_messages, RawSessionMessagesRequest};
use rusqlite::params;

fn identify_raw_row(conn: &Connection, row_id: i64, host: &str, transcript: &str) {
    let session_mode = if host == "codex-cli" {
        "interactive"
    } else {
        "unknown"
    };
    identify_raw_row_with_mode(conn, row_id, host, session_mode, transcript);
}

fn identify_raw_row_with_mode(
    conn: &Connection,
    row_id: i64,
    host: &str,
    session_mode: &str,
    transcript: &str,
) {
    let (root, project, session): (String, String, String) = conn
        .query_row(
            "SELECT source_root, project, session_id FROM raw_messages WHERE id = ?1",
            [row_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO raw_session_identities
         (source_root, transcript_path, host, session_mode, fallback_session_id,
          canonical_session_id, project, legacy_project, status,
          contract_version, observed_mtime_ns, observed_size_bytes,
          first_seen_at_epoch, last_seen_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?6, 'active', 1, 1, 1, 1, 1)",
        params![root, transcript, host, session_mode, session, project],
    )
    .unwrap();
    let identity_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE raw_messages
         SET transcript_identity_id = ?1, transcript_record_ordinal = 1
         WHERE id = ?2",
        params![identity_id, row_id],
    )
    .unwrap();
}

fn insert_unresolved_transcript(
    conn: &Connection,
    session_id: &str,
    project: &str,
    content: &str,
    epoch: i64,
) -> i64 {
    let row_id = insert_at_epoch(conn, session_id, project, ROLE_USER, content, epoch);
    conn.execute(
        "UPDATE raw_messages
         SET source = 'transcript', event_time_source = 'transcript_event'
         WHERE id = ?1",
        [row_id],
    )
    .unwrap();
    row_id
}

fn exact_messages_request(session_id: &str) -> RawSessionMessagesRequest {
    RawSessionMessagesRequest {
        host: "codex-cli".to_string(),
        source_root: "local".to_string(),
        project: "/repo".to_string(),
        session_id: session_id.to_string(),
        limit: 20,
        cursor: None,
    }
}

fn assert_listing_reports_session(
    listing: &RawSessionListing,
    session_id: &str,
    host: Option<&str>,
) {
    assert!(
        listing.excluded_legacy_identities.iter().any(|identity| {
            identity.session_id == session_id && identity.host.as_deref() == host
        }),
        "expected skipped identity {session_id:?} host={host:?}, got {:?}",
        listing.excluded_legacy_identities
    );
}

#[test]
fn list_sessions_skips_conflicting_modes_without_aborting_the_archive() {
    let conn = setup_conn();
    let first = insert_at_epoch(&conn, "shared", "/repo", ROLE_USER, "first", 100);
    let second = insert_at_epoch(&conn, "shared", "/repo", ROLE_ASSISTANT, "second", 200);
    identify_raw_row_with_mode(
        &conn,
        first,
        "codex-cli",
        "interactive",
        "/tmp/.codex/sessions/first.jsonl",
    );
    identify_raw_row_with_mode(
        &conn,
        second,
        "codex-cli",
        "unattended",
        "/tmp/.codex/sessions/second.jsonl",
    );
    let healthy = insert_at_epoch(&conn, "healthy", "/repo", ROLE_USER, "healthy", 300);
    identify_raw_row(
        &conn,
        healthy,
        "codex-cli",
        "/tmp/.codex/sessions/healthy.jsonl",
    );

    let listing = list_sessions_with_exclusions(&conn, &RawSessionQuery::default())
        .expect("mode conflicts must skip that session instead of aborting listing");
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].session_id, "healthy");
    assert_eq!(listing.excluded_legacy_sessions, 1);
    assert_listing_reports_session(&listing, "shared", Some("codex-cli"));
}

#[test]
fn list_sessions_separates_same_selector_across_hosts() {
    let conn = setup_conn();
    let codex_row = insert_at_epoch(&conn, "shared", "/repo", ROLE_USER, "codex", 100);
    let claude_row = insert_at_epoch(&conn, "shared", "/repo", ROLE_USER, "claude", 110);
    identify_raw_row(
        &conn,
        codex_row,
        "codex-cli",
        "/tmp/.codex/sessions/shared.jsonl",
    );
    identify_raw_row(
        &conn,
        claude_row,
        "claude-code",
        "/tmp/.claude/projects/repo/shared.jsonl",
    );

    let sessions = list_sessions(&conn, &RawSessionQuery::default()).unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].host, "codex-cli");
    assert_eq!(sessions[0].session_mode, "interactive");
    assert_eq!(sessions[1].host, "claude-code");
    assert_eq!(sessions[1].session_mode, "unknown");
    assert_ne!(sessions[0].session_ref, sessions[1].session_ref);
    assert_ne!(sessions[0].content_hash, sessions[1].content_hash);
}

#[test]
fn list_sessions_keeps_unbound_hook_fallbacks_outside_the_transcript_contract() {
    let conn = setup_conn();
    let identified = insert_at_epoch(&conn, "shared", "/repo", ROLE_USER, "question", 100);
    identify_raw_row(
        &conn,
        identified,
        "codex-cli",
        "/tmp/.codex/sessions/shared.jsonl",
    );
    insert_raw_message(
        &conn,
        "shared",
        "/repo",
        ROLE_ASSISTANT,
        "hook fallback",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap();
    insert_raw_message(
        &conn,
        "hook-only",
        "/repo",
        ROLE_ASSISTANT,
        "partial fallback",
        SOURCE_HOOK,
        None,
        None,
    )
    .unwrap();

    let sessions = list_sessions_with_exclusions(&conn, &RawSessionQuery::default()).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "shared");
    assert_eq!(sessions[0].message_count, 1);
    assert_eq!(sessions[0].user_message_count, 1);
    assert_eq!(sessions[0].assistant_message_count, 0);
}

#[test]
fn list_sessions_keeps_preidentity_legacy_rows_outside_the_transcript_contract() {
    let conn = setup_conn();
    let identified = insert_at_epoch(&conn, "current", "/repo", ROLE_USER, "current", 200);
    identify_raw_row(
        &conn,
        identified,
        "codex-cli",
        "/tmp/.codex/sessions/current.jsonl",
    );
    let legacy = insert_at_epoch(&conn, "legacy", "/old", ROLE_USER, "legacy", 100);
    conn.execute(
        "UPDATE raw_messages
         SET source = 'transcript', event_time_source = 'legacy_unknown'
         WHERE id = ?1",
        [legacy],
    )
    .unwrap();

    let sessions = list_sessions_with_exclusions(&conn, &RawSessionQuery::default()).unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "current");
    assert_eq!(sessions.excluded_legacy_rows, 1);
    assert_eq!(sessions.excluded_legacy_sessions, 1);
    assert_listing_reports_session(&sessions, "legacy", None);
    let output = serde_json::to_value(build_session_listing_json(
        &RawSessionQuery::default(),
        sessions,
    ))
    .unwrap();
    assert_eq!(output["excluded_legacy_rows"], 1);
    assert_eq!(output["excluded_legacy_sessions"], 1);
    assert_eq!(
        output["excluded_legacy_identities"][0]["session_id"],
        "legacy"
    );
    assert!(output["excluded_legacy_identities"][0]["host"].is_null());
}

#[test]
fn list_sessions_hash_is_stable_and_changes_with_an_identified_occurrence() {
    let conn = setup_conn();
    let first = insert_at_epoch(&conn, "s1", "/repo", ROLE_USER, "first", 100);
    identify_raw_row(&conn, first, "codex-cli", "/tmp/.codex/sessions/s1.jsonl");
    let before = list_sessions(&conn, &RawSessionQuery::default()).unwrap();
    let repeated = list_sessions(&conn, &RawSessionQuery::default()).unwrap();
    assert_eq!(before[0].session_ref, repeated[0].session_ref);
    assert_eq!(before[0].content_hash, repeated[0].content_hash);

    let second = insert_at_epoch(&conn, "s1", "/repo", ROLE_ASSISTANT, "second", 200);
    let identity_id: i64 = conn
        .query_row(
            "SELECT transcript_identity_id FROM raw_messages WHERE id = ?1",
            [first],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "UPDATE raw_messages
         SET transcript_identity_id = ?1, transcript_record_ordinal = 2
         WHERE id = ?2",
        params![identity_id, second],
    )
    .unwrap();
    let after = list_sessions(&conn, &RawSessionQuery::default()).unwrap();
    assert_eq!(before[0].session_ref, after[0].session_ref);
    assert_ne!(before[0].content_hash, after[0].content_hash);
    assert_eq!(after[0].message_count, 2);

    conn.execute(
        "UPDATE raw_messages SET transcript_record_ordinal = 3 WHERE id = ?1",
        [second],
    )
    .unwrap();
    let rekeyed = list_sessions(&conn, &RawSessionQuery::default()).unwrap();
    assert_ne!(after[0].content_hash, rekeyed[0].content_hash);
}

#[test]
fn list_sessions_latest_returns_healthy_sessions_when_unresolved_rows_exist() {
    let conn = setup_conn();
    insert_at_epoch(&conn, "old", "/repo", ROLE_USER, "old", 100);
    insert_at_epoch(&conn, "new", "/repo", ROLE_USER, "new", 200);
    identify_raw_sessions(&conn, "codex-cli");
    let older_unresolved =
        insert_unresolved_transcript(&conn, "old-unresolved", "/repo", "older missing", 50);
    let interleaved =
        insert_unresolved_transcript(&conn, "interleaved-unresolved", "/repo", "interleaved", 150);
    let newer_unresolved =
        insert_unresolved_transcript(&conn, "missing-host", "/repo", "missing", 300);
    assert!(older_unresolved > 0 && interleaved > 0 && newer_unresolved > 0);

    let listing = list_sessions_with_exclusions(&conn, &RawSessionQuery::default())
        .expect("unresolved rows must not abort the archive listing");
    assert_eq!(
        listing
            .sessions
            .iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["old", "new"]
    );
    assert_eq!(listing.excluded_legacy_rows, 3);
    assert_eq!(listing.excluded_legacy_sessions, 3);
    assert_listing_reports_session(&listing, "old-unresolved", None);
    assert_listing_reports_session(&listing, "interleaved-unresolved", None);
    assert_listing_reports_session(&listing, "missing-host", None);

    let latest = list_sessions_with_exclusions(
        &conn,
        &RawSessionQuery {
            latest: Some(1),
            ..RawSessionQuery::default()
        },
    )
    .expect("latest must ignore unresolved rows while selecting the newest healthy session");
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].session_id, "new");
    assert!(latest.excluded_legacy_rows >= 1);
    assert_listing_reports_session(&latest, "missing-host", None);
    assert_listing_reports_session(&latest, "old-unresolved", None);

    let error = query_raw_session_messages(&conn, &exact_messages_request("missing-host"))
        .expect_err("exact identity reads must stay fail-closed");
    assert!(error
        .to_string()
        .contains("provenance is missing or conflicted"));
    assert!(!error.to_string().contains("re-ingest"));
}

#[test]
fn list_sessions_skips_a_mixed_session_when_any_row_is_globally_unresolved() {
    let conn = setup_conn();
    let identified = insert_at_epoch(&conn, "mixed", "/repo", ROLE_USER, "identified", 100);
    let other = insert_at_epoch(&conn, "healthy", "/repo", ROLE_USER, "healthy", 200);
    identify_raw_row(
        &conn,
        identified,
        "codex-cli",
        "/tmp/.codex/sessions/mixed.jsonl",
    );
    identify_raw_row(
        &conn,
        other,
        "codex-cli",
        "/tmp/.codex/sessions/healthy.jsonl",
    );
    insert_unresolved_transcript(&conn, "mixed", "/repo", "unresolved sibling", 110);

    let listing = list_sessions_with_exclusions(&conn, &RawSessionQuery::default())
        .expect("a mixed unresolved session must not hide unrelated healthy sessions");
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].session_id, "healthy");
    assert_listing_reports_session(&listing, "mixed", None);
    let error = query_raw_session_messages(&conn, &exact_messages_request("mixed"))
        .expect_err("mixed unresolved exact selector must stay fail-closed");
    assert!(error
        .to_string()
        .contains("provenance is missing or conflicted"));
}

#[test]
fn list_sessions_reads_only_selected_bounded_samples() {
    let conn = setup_conn();
    let old = insert_at_epoch(&conn, "old", "/repo", ROLE_USER, "old", 100);
    let first = insert_at_epoch(&conn, "new", "/repo", ROLE_USER, "first", 200);
    let second = insert_at_epoch(&conn, "new", "/repo", ROLE_USER, "second", 210);
    identify_raw_sessions(&conn, "codex-cli");
    conn.execute(
        "UPDATE raw_messages SET content = CAST(X'80' AS BLOB) WHERE id IN (?1, ?2)",
        params![old, second],
    )
    .unwrap();

    let sessions = list_sessions(
        &conn,
        &RawSessionQuery {
            sample_user_messages: 1,
            latest: Some(1),
            ..RawSessionQuery::default()
        },
    )
    .expect("discarded and over-limit sample content must not be read");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "new");
    assert_eq!(sessions[0].message_count, 2);
    assert_eq!(sessions[0].user_message_samples, vec!["first"]);
    assert!(first > 0);
}
