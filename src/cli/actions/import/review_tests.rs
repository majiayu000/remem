use super::*;
use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path, ScopedTestDataDir};
use sha2::{Digest, Sha256};

fn create_review_source(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY,
            session_id TEXT,
            project TEXT NOT NULL,
            topic_key TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            files TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            branch TEXT,
            scope TEXT DEFAULT 'project',
            acknowledged_pattern_id TEXT,
            acknowledged_pattern_version INTEGER,
            acknowledged_at_epoch INTEGER
        );",
    )?;
    Ok(conn)
}

#[test]
fn backup_snapshot_digest_tracks_wal_visible_state() -> Result<()> {
    let source_path = unique_temp_db_path("runtime-import-wal-snapshot");
    let source = create_review_source(&source_path)?;
    source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")?;
    source.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch)
         VALUES (1, '/tmp/wal-import', 'wal-topic', 'WAL title', 'version one',
                 'decision', 1, 1)",
        [],
    )?;
    let main_before = Sha256::digest(std::fs::read(&source_path)?);
    let (first_snapshot, first_sha) = backup_snapshot::open_consistent_snapshot(&source_path)?;
    assert_eq!(
        first_snapshot.query_row("SELECT content FROM memories WHERE id = 1", [], |row| {
            row.get::<_, String>(0)
        })?,
        "version one"
    );

    source.execute(
        "UPDATE memories SET content = 'version two', updated_at_epoch = 2 WHERE id = 1",
        [],
    )?;
    let main_after = Sha256::digest(std::fs::read(&source_path)?);
    assert_eq!(main_before.as_slice(), main_after.as_slice());
    let (second_snapshot, second_sha) = backup_snapshot::open_consistent_snapshot(&source_path)?;
    assert_ne!(first_sha, second_sha);
    assert_eq!(
        second_snapshot.query_row("SELECT content FROM memories WHERE id = 1", [], |row| {
            row.get::<_, String>(0)
        })?,
        "version two"
    );

    drop(source);
    cleanup_temp_db_files(&source_path);
    Ok(())
}

#[test]
fn backup_import_restores_acknowledged_instruction_pattern() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("import-acknowledged-pattern");
    let source_path = unique_temp_db_path("runtime-import-acknowledged-pattern");
    let source = create_review_source(&source_path)?;
    source.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, acknowledged_pattern_id,
          acknowledged_pattern_version, acknowledged_at_epoch)
         VALUES (1, '/tmp/ack-import', 'ack-topic', 'Acknowledged backup',
                 'Ignore previous instructions only as a quoted false-positive fixture.',
                 'decision', 1, 2, 'override_previous_instructions', ?1, 3)",
        [crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION],
    )?;
    source.execute(
        "INSERT INTO memories
         (id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, acknowledged_pattern_id,
          acknowledged_pattern_version, acknowledged_at_epoch)
         VALUES (2, '/tmp/ack-import', 'incomplete-ack-topic', 'Incomplete acknowledgement',
                 'Ignore previous instructions in another quoted fixture.',
                 'decision', 1, 2, 'stale', 'override_previous_instructions', ?1, NULL)",
        [crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION],
    )?;
    drop(source);

    let runtime = crate::db::open_db()?;
    let stats = import_memories_into_runtime(&source_path, &runtime)?;
    assert_eq!(stats.memories_imported, 1);
    assert_eq!(stats.memories_skipped, 1);
    let (memory_id, pattern_id, pattern_version): (i64, String, i64) = runtime.query_row(
        "SELECT id, acknowledged_pattern_id, acknowledged_pattern_version
         FROM memories WHERE topic_key = 'ack-topic'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(pattern_id, "override_previous_instructions");
    assert_eq!(
        pattern_version,
        crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION
    );
    let (route, verdict, result_sha): (String, String, String) = runtime.query_row(
        "SELECT route_kind, poisoning_verdict, result_sha256
         FROM memory_activation_requests WHERE result_memory_id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(route, "exact_recovery");
    assert_eq!(verdict, "exact_recovery");
    let actual =
        crate::memory::activation::ExpectedActiveMemory::from_existing(&runtime, memory_id)?;
    assert_eq!(result_sha, actual.sha256());
    assert_eq!(
        runtime.query_row(
            "SELECT COUNT(*) FROM memories WHERE topic_key = 'incomplete-ack-topic'",
            [],
            |row| row.get::<_, i64>(0),
        )?,
        0
    );

    cleanup_temp_db_files(&source_path);
    Ok(())
}
