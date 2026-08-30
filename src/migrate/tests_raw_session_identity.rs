use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::MIGRATIONS;

const V071: i64 = 71;
const V091: i64 = 91;

fn migration_v071() -> Result<&'static super::types::Migration> {
    MIGRATIONS
        .iter()
        .find(|migration| migration.version == V071)
        .context("v071 migration is missing")
}

fn pre_v071() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON")?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version < V071)
    {
        conn.execute_batch(migration.sql)?;
    }
    Ok(conn)
}

fn pre_v091() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON")?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version < V091)
    {
        conn.execute_batch(migration.sql)?;
    }
    Ok(conn)
}

fn insert_pre_v091_conflict(
    conn: &Connection,
    transcript_path: &str,
    fallback_session_id: &str,
    reason: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO raw_session_identities
         (source_root, transcript_path, fallback_session_id,
          canonical_session_id, project, legacy_project, status, conflict_reason,
          contract_version, observed_mtime_ns, observed_size_bytes,
          first_seen_at_epoch, last_seen_at_epoch)
         VALUES ('local', ?1, ?2, ?2, '/repo', '/repo', 'conflict', ?3,
                 1, 1, 1, 1, 1)",
        params![transcript_path, fallback_session_id, reason],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_pre_v091_metadata_claim(
    conn: &Connection,
    identity_id: i64,
    claimed_session_id: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO raw_session_identity_claims
         (transcript_identity_id, claimed_session_id, identity_source,
          first_seen_at_epoch, last_seen_at_epoch)
         VALUES (?1, ?2, 'transcript_metadata', 1, 1)",
        params![identity_id, claimed_session_id],
    )?;
    Ok(())
}

#[test]
fn v071_is_registered_and_named_stably() -> Result<()> {
    let migration = migration_v071()?;
    assert_eq!(migration.version, V071);
    assert_eq!(migration.name, "raw_session_identity");
    Ok(())
}

#[test]
fn v071_preserves_raw_rows_and_fts() -> Result<()> {
    let conn = pre_v071()?;
    conn.execute(
        "INSERT INTO raw_messages (
            id, session_id, project, role, content, content_hash, source,
            created_at_epoch, source_root
         ) VALUES (41, 'fallback', 'project', 'user', 'searchable v071',
                   'hash-41', 'transcript', 100, 'local')",
        [],
    )?;

    let migration = migration_v071()?;
    conn.execute_batch(migration.sql)?;

    let row: (String, String, Option<i64>, Option<i64>) = conn.query_row(
        "SELECT content, event_time_source, transcript_identity_id,
                transcript_record_ordinal
         FROM raw_messages WHERE id = 41",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        row,
        (
            "searchable v071".into(),
            "legacy_unknown".into(),
            None,
            None
        )
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_messages_fts
             WHERE raw_messages_fts MATCH 'searchable'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        1
    );
    Ok(())
}

#[test]
fn v071_occurrence_key_preserves_repeated_turns_and_replay_idempotency() -> Result<()> {
    let conn = pre_v071()?;
    let migration = migration_v071()?;
    conn.execute_batch(migration.sql)?;
    conn.execute(
        "INSERT INTO raw_session_identities (
            source_root, transcript_path, fallback_session_id,
            canonical_session_id, project, legacy_project, status,
            contract_version, observed_mtime_ns, observed_size_bytes,
            first_seen_at_epoch, last_seen_at_epoch
         ) VALUES ('local', '/tmp/repeated.jsonl', 'fallback', 'canonical',
                   'project', 'legacy', 'active', 1, 1, 1, 1, 1)",
        [],
    )?;
    let identity_id = conn.last_insert_rowid();

    for ordinal in [7_i64, 8, 7] {
        conn.execute(
            "INSERT OR IGNORE INTO raw_messages (
                session_id, project, role, content, content_hash, source,
                created_at_epoch, source_root, event_time_source,
                transcript_identity_id, transcript_record_ordinal
             ) VALUES ('canonical', 'project', 'user', 'same', 'same-hash',
                       'transcript', 100, 'local', 'transcript_event', ?1, ?2)",
            params![identity_id, ordinal],
        )?;
    }

    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_messages WHERE transcript_identity_id = ?1",
            [identity_id],
            |row| row.get::<_, i64>(0)
        )?,
        2
    );
    Ok(())
}

#[test]
fn v071_enforces_identity_foreign_keys_and_closed_values() -> Result<()> {
    let conn = pre_v071()?;
    let migration = migration_v071()?;
    conn.execute_batch(migration.sql)?;

    let claim_fk_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('raw_session_identity_claims')
         WHERE \"table\" = 'raw_session_identities'
           AND \"from\" = 'transcript_identity_id'
           AND on_delete = 'RESTRICT'",
        [],
        |row| row.get(0),
    )?;
    let raw_fk_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('raw_messages')
         WHERE \"table\" = 'raw_session_identities'
           AND \"from\" = 'transcript_identity_id'
           AND on_delete = 'RESTRICT'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(claim_fk_count, 1);
    assert_eq!(raw_fk_count, 1);

    let identity_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type = 'table' AND name = 'raw_session_identities'",
        [],
        |row| row.get(0),
    )?;
    let claim_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type = 'table' AND name = 'raw_session_identity_claims'",
        [],
        |row| row.get(0),
    )?;
    let raw_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type = 'table' AND name = 'raw_messages'",
        [],
        |row| row.get(0),
    )?;
    assert!(identity_sql.contains("CHECK(status IN ('active', 'conflict'))"));
    assert!(identity_sql
        .contains("CHECK(event_index_status IN ('pending', 'since_indexed', 'complete'))"));
    assert!(claim_sql
        .contains("CHECK(identity_source IN ('transcript_metadata', 'filename_fallback'))"));
    assert!(raw_sql.contains("'transcript_event', 'ingest_fallback', 'legacy_unknown'"));
    assert!(
        raw_sql.contains("transcript_identity_id IS NULL AND transcript_record_ordinal IS NULL")
    );
    let occurrence_index_sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type = 'index' AND name = 'idx_raw_messages_transcript_occurrence'",
        [],
        |row| row.get(0),
    )?;
    assert!(occurrence_index_sql.contains("transcript_identity_id, transcript_record_ordinal"));
    assert!(!occurrence_index_sql.contains("project"));
    assert!(!occurrence_index_sql.contains("session_id"));
    Ok(())
}

#[test]
fn v091_backfills_known_hosts_and_leaves_unknown_paths_explicit() -> Result<()> {
    let conn = pre_v091()?;
    for (index, path) in [
        "/tmp/.claude/projects/repo/a.jsonl",
        "/tmp/.codex/sessions/2026/08/b.jsonl",
        "/tmp/.cursor/projects/repo/c.jsonl",
        r"C:\Users\me\.claude\projects\repo\windows-a.jsonl",
        r"C:\Users\me\.codex\sessions\2026\08\windows-b.jsonl",
        r"C:\Users\me\.cursor\projects\repo\windows-c.jsonl",
        "/tmp/unclassified/d.jsonl",
        "/tmp/.codex/sessions/.claude/projects/ambiguous.jsonl",
        "/tmp/.CLAUDE/projects/case-near-miss.jsonl",
        "/tmp/.CoDeX/sessions/case-near-miss.jsonl",
        "/tmp/.CURSOR/case-near-miss.jsonl",
        "/tmp/.claude/projectss/component-near-miss.jsonl",
        ".claude/projects/relative-valid.jsonl",
    ]
    .into_iter()
    .enumerate()
    {
        conn.execute(
            "INSERT INTO raw_session_identities
             (source_root, transcript_path, fallback_session_id,
              canonical_session_id, project, legacy_project, status,
              contract_version, observed_mtime_ns, observed_size_bytes,
              first_seen_at_epoch, last_seen_at_epoch)
             VALUES ('local', ?1, ?2, ?2, '/repo', '/repo', 'active', 1, 1, 1, 1, 1)",
            params![path, format!("s{index}")],
        )?;
    }

    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == V091)
        .context("v091 migration is missing")?;
    assert_eq!(migration.name, "raw_session_host");
    conn.execute_batch(migration.sql)?;
    let hosts = {
        let mut statement = conn.prepare("SELECT host FROM raw_session_identities ORDER BY id")?;
        let rows = statement
            .query_map([], |row| row.get::<_, Option<String>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    assert_eq!(
        hosts,
        vec![
            Some("claude-code".to_string()),
            Some("codex-cli".to_string()),
            Some("cursor".to_string()),
            Some("claude-code".to_string()),
            Some("codex-cli".to_string()),
            Some("cursor".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            Some("claude-code".to_string()),
        ]
    );
    assert!(conn
        .execute(
            "UPDATE raw_session_identities SET host = 'guessed' WHERE id = 1",
            [],
        )
        .is_err());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_session_identities WHERE session_mode = 'unknown'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        13
    );
    assert!(conn
        .execute(
            "UPDATE raw_session_identities SET session_mode = 'guessed' WHERE id = 1",
            [],
        )
        .is_err());
    Ok(())
}

#[test]
fn v091_re_resolves_only_host_scoped_metadata_conflicts() -> Result<()> {
    let conn = pre_v091()?;
    let cross_claude = insert_pre_v091_conflict(
        &conn,
        "/tmp/.claude/projects/repo/cross.jsonl",
        "shared",
        "conflicting_metadata_claims",
    )?;
    let cross_codex = insert_pre_v091_conflict(
        &conn,
        "/tmp/.codex/sessions/cross.jsonl",
        "shared",
        "conflicting_metadata_claims",
    )?;
    insert_pre_v091_metadata_claim(&conn, cross_claude, "claude-id")?;
    insert_pre_v091_metadata_claim(&conn, cross_codex, "codex-id")?;
    insert_pre_v091_conflict(
        &conn,
        "/tmp/.cursor/projects/repo/cross.jsonl",
        "shared",
        "conflicting_metadata_claims",
    )?;

    let same_a = insert_pre_v091_conflict(
        &conn,
        "/tmp/.claude/projects/repo/same-a.jsonl",
        "same-host",
        "conflicting_metadata_claims",
    )?;
    let same_b = insert_pre_v091_conflict(
        &conn,
        "/tmp/.claude/projects/repo/same-b.jsonl",
        "same-host",
        "conflicting_metadata_claims",
    )?;
    insert_pre_v091_metadata_claim(&conn, same_a, "claude-a")?;
    insert_pre_v091_metadata_claim(&conn, same_b, "claude-b")?;
    insert_pre_v091_conflict(
        &conn,
        "/tmp/.codex/sessions/sticky.jsonl",
        "sticky",
        "stable_occurrence_mismatch",
    )?;
    insert_pre_v091_conflict(
        &conn,
        "/tmp/.codex/sessions/.claude/projects/ambiguous.jsonl",
        "ambiguous",
        "conflicting_metadata_claims",
    )?;

    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == V091)
        .context("v091 migration is missing")?;
    conn.execute_batch(migration.sql)?;
    let load = |path: &str| -> Result<(Option<String>, String, String, Option<String>)> {
        Ok(conn.query_row(
            "SELECT host, canonical_session_id, status, conflict_reason
             FROM raw_session_identities WHERE transcript_path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?)
    };

    assert_eq!(
        load("/tmp/.claude/projects/repo/cross.jsonl")?,
        (
            Some("claude-code".into()),
            "claude-id".into(),
            "active".into(),
            None
        )
    );
    assert_eq!(
        load("/tmp/.codex/sessions/cross.jsonl")?,
        (
            Some("codex-cli".into()),
            "codex-id".into(),
            "active".into(),
            None
        )
    );
    assert_eq!(
        load("/tmp/.cursor/projects/repo/cross.jsonl")?,
        (
            Some("cursor".into()),
            "shared".into(),
            "active".into(),
            None
        )
    );
    for path in [
        "/tmp/.claude/projects/repo/same-a.jsonl",
        "/tmp/.claude/projects/repo/same-b.jsonl",
    ] {
        let row = load(path)?;
        assert_eq!(row.0, Some("claude-code".into()));
        assert_eq!(row.2, "conflict");
        assert_eq!(row.3.as_deref(), Some("conflicting_metadata_claims"));
    }
    let sticky = load("/tmp/.codex/sessions/sticky.jsonl")?;
    assert_eq!(sticky.2, "conflict");
    assert_eq!(sticky.3.as_deref(), Some("stable_occurrence_mismatch"));
    let ambiguous = load("/tmp/.codex/sessions/.claude/projects/ambiguous.jsonl")?;
    assert_eq!(ambiguous.0, None);
    assert_eq!(ambiguous.2, "conflict");
    Ok(())
}
