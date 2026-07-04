use anyhow::Result;
use rusqlite::{params, Connection};

use super::MIGRATIONS;

#[test]
fn multimodel_key_migration_preserves_existing_profile() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 58) {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute(
        "INSERT INTO memories(project, topic_key, title, content, memory_type,
            created_at_epoch, updated_at_epoch, status)
         VALUES ('proj', 'embedding-key', 'Embedding migration',
            'Existing vectors must survive profile-key migration.',
            'decision', 1, 1, 'active')",
        [],
    )?;
    let memory_id = conn.last_insert_rowid();
    let old_blob = vec![0u8; 3 * std::mem::size_of::<f32>()];
    conn.execute(
        "INSERT INTO memory_embeddings
            (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (?1, ?2, 3, 'old-model', 'old-hash', 1)",
        params![memory_id, old_blob],
    )?;

    let migration = MIGRATIONS
        .iter()
        .find(|migration| migration.version == 58)
        .expect("v058 migration should exist");
    conn.execute_batch(migration.sql)?;
    let new_blob = vec![0u8; 4 * std::mem::size_of::<f32>()];
    conn.execute(
        "INSERT INTO memory_embeddings
            (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (?1, ?2, 4, 'new-model', 'new-hash', 2)",
        params![memory_id, new_blob],
    )?;

    let rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings WHERE memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(rows, 2);
    for (model, dimensions) in [("old-model", 3_i64), ("new-model", 4_i64)] {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM memory_embeddings
                 WHERE memory_id = ?1 AND model = ?2 AND dimensions = ?3",
                params![memory_id, model, dimensions],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(exists, "{model}/{dimensions} embedding row should exist");
    }
    Ok(())
}
