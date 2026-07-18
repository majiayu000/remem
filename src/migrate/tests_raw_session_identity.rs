use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::MIGRATIONS;

const V071: i64 = 71;

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

#[test]
fn v071_is_latest_and_named_stably() -> Result<()> {
    let migration = MIGRATIONS.last().context("v071 migration is missing")?;
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

    let migration = MIGRATIONS.last().context("v071 migration is missing")?;
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
    let migration = MIGRATIONS.last().context("v071 migration is missing")?;
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
