use anyhow::Result;
use rusqlite::{params, Connection};

use super::{run_migrations, MIGRATIONS};

fn setup_v040_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 40)
    {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute_batch(
        "CREATE TABLE _schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at_epoch INTEGER NOT NULL
        );",
    )?;
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 40)
    {
        conn.execute(
            "INSERT INTO _schema_migrations (version, name, applied_at_epoch)
             VALUES (?1, ?2, ?3)",
            params![migration.version, migration.name, 1_700_000_000_i64],
        )?;
    }
    Ok(conn)
}

#[test]
fn content_identity_migration_backfills_legacy_hashes() -> Result<()> {
    let conn = setup_v040_db()?;
    let raw_content = "legacy searchable raw message";
    conn.execute(
        "INSERT INTO raw_messages
         (session_id, project, role, content, content_hash, source, created_at_epoch)
         VALUES ('session-a', '/tmp/remem-content-id', 'user', ?1, ?2, 'hook', 100)",
        params![
            raw_content,
            crate::db::legacy_content_identity_hash(raw_content.as_bytes())
        ],
    )?;

    let now = 1_700_000_000_i64;
    let host_id: i64 =
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
         VALUES ('/tmp/remem-content-id', ?1, ?1)",
        [now],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/tmp/remem-content-id', 'tmp-remem-content-id', ?2, ?2)",
        params![workspace_id, now],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, 'session-a', ?4, 'active')",
        params![host_id, workspace_id, project_id, now],
    )?;
    let session_row_id = conn.last_insert_rowid();

    let inline_content = "legacy inline captured content";
    conn.execute(
        "INSERT INTO captured_events
         (host_id, workspace_id, project_id, session_row_id, session_id, event_id,
          event_type, content_text, content_hash, token_estimate, retention_class,
          created_at_epoch, inserted_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'session-a', 'inline-event', 'message',
                 ?5, ?6, 1, 'raw_keep', ?7, ?7)",
        params![
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            inline_content,
            crate::db::legacy_content_identity_hash(inline_content.as_bytes()),
            now
        ],
    )?;

    let blob_bytes = b"legacy full blob bytes for v041";
    conn.execute(
        "INSERT INTO event_blobs
         (content_hash, content_encoding, content_bytes, original_bytes, stored_bytes, created_at_epoch)
         VALUES (?1, 'plain', ?2, ?3, ?3, ?4)",
        params![
            crate::db::legacy_content_identity_hash(blob_bytes),
            blob_bytes,
            blob_bytes.len() as i64,
            now
        ],
    )?;
    let blob_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captured_events
         (host_id, workspace_id, project_id, session_row_id, session_id, event_id,
          event_type, content_text, content_blob_id, content_hash, token_estimate,
          retention_class, created_at_epoch, inserted_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'session-a', 'blob-event', 'tool_result',
                 'compact preview should not be hashed', ?5, ?6, 10, 'raw_compact', ?7, ?7)",
        params![
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            blob_id,
            crate::db::legacy_content_identity_hash(blob_bytes),
            now
        ],
    )?;

    run_migrations(&conn)?;

    let raw_hash: String = conn.query_row("SELECT content_hash FROM raw_messages", [], |row| {
        row.get(0)
    })?;
    let blob_hash: String =
        conn.query_row("SELECT content_hash FROM event_blobs", [], |row| row.get(0))?;
    let inline_hash: String = conn.query_row(
        "SELECT content_hash FROM captured_events WHERE event_id = 'inline-event'",
        [],
        |row| row.get(0),
    )?;
    let blob_event_hash: String = conn.query_row(
        "SELECT content_hash FROM captured_events WHERE event_id = 'blob-event'",
        [],
        |row| row.get(0),
    )?;
    let fts_hits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages_fts WHERE raw_messages_fts MATCH 'searchable'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(
        raw_hash,
        crate::db::content_identity_hash(raw_content.as_bytes())
    );
    assert_eq!(blob_hash, crate::db::content_identity_hash(blob_bytes));
    assert_eq!(
        inline_hash,
        crate::db::content_identity_hash(inline_content.as_bytes())
    );
    assert_eq!(
        blob_event_hash,
        crate::db::content_identity_hash(blob_bytes)
    );
    assert_eq!(fts_hits, 1);
    Ok(())
}

#[test]
fn content_identity_migration_collapses_mixed_raw_duplicates() -> Result<()> {
    let conn = setup_v040_db()?;
    let content = "mixed legacy and sha duplicate raw message";
    let legacy_hash = crate::db::legacy_content_identity_hash(content.as_bytes());
    let sha_hash = crate::db::content_identity_hash(content.as_bytes());

    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, created_at_epoch)
         VALUES (1, 'session-a', '/tmp/remem-content-id', 'user', ?1, ?2, 'hook', 100)",
        params![content, legacy_hash],
    )?;
    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, created_at_epoch)
         VALUES (2, 'session-a', '/tmp/remem-content-id', 'user', ?1, ?2, 'hook', 101)",
        params![content, sha_hash],
    )?;

    run_migrations(&conn)?;

    let rows: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, content_hash FROM raw_messages ORDER BY id")?;
        let mapped = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row?);
        }
        rows
    };
    let fts_hits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM raw_messages_fts WHERE raw_messages_fts MATCH 'duplicate'",
        [],
        |row| row.get(0),
    )?;

    assert_eq!(rows, vec![(1, sha_hash)]);
    assert_eq!(fts_hits, 1);
    Ok(())
}
