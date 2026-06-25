use anyhow::Result;
use rusqlite::Connection;

use super::MIGRATIONS;

fn apply_workstream_identity_migration(conn: &Connection) -> Result<()> {
    let migration = &MIGRATIONS[52];
    conn.execute_batch(migration.sql)?;
    super::run::run_post_migration_hook(conn, migration.version, migration.name)?;
    Ok(())
}

#[test]
fn workstream_identity_migration_backfills_alias_history() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    for migration in &MIGRATIONS[..52] {
        conn.execute_batch(migration.sql)?;
    }

    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key)
         VALUES ('test/proj', 'agent-workflow Skill 生命周期工作流', 'active',
                 1700000000, 1700000100, 'test/proj', 'test/proj', 'repo', 'test/proj')",
        [],
    )?;
    apply_workstream_identity_migration(&conn)?;

    let identity_key: String = conn.query_row(
        "SELECT identity_key FROM workstreams WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(identity_key, "ws_1");

    let alias: (String, String, i64, i64) = conn.query_row(
        "SELECT title, normalized_title, first_seen_epoch, last_seen_epoch
         FROM workstream_aliases WHERE workstream_id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(alias.0, "agent-workflow Skill 生命周期工作流");
    assert_eq!(alias.1, "agent workflow skill 生命周期工作流");
    assert_eq!(alias.2, 1700000000);
    assert_eq!(alias.3, 1700000100);

    let source: (String, Option<String>, i64) = conn.query_row(
        "SELECT was.source, was.memory_session_id, was.source_workstream_id
         FROM workstream_alias_sources was
         JOIN workstream_aliases wa ON wa.id = was.alias_id
         WHERE wa.workstream_id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(source, ("migration".to_string(), None, 1));

    Ok(())
}

#[test]
fn workstream_identity_migration_collapses_alias_whitespace_like_runtime() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    for migration in &MIGRATIONS[..52] {
        conn.execute_batch(migration.sql)?;
    }

    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key)
         VALUES ('test/proj', ?1, 'active',
                 1700000000, 1700000100, 'test/proj', 'test/proj', 'repo', 'test/proj')",
        ["flowguard|\trun-guard {Skill}<生命周期工作流>\n"],
    )?;
    apply_workstream_identity_migration(&conn)?;

    let normalized_title: String = conn.query_row(
        "SELECT normalized_title FROM workstream_aliases WHERE workstream_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(normalized_title, "flowguard run guard skill 生命周期工作流");

    let source_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM workstream_alias_sources was
         JOIN workstream_aliases wa ON wa.id = was.alias_id
         WHERE wa.normalized_title = ?1",
        ["flowguard run guard skill 生命周期工作流"],
        |row| row.get(0),
    )?;
    assert_eq!(source_count, 1);

    Ok(())
}

#[test]
fn workstream_identity_migration_uses_runtime_unicode_normalizer() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    for migration in &MIGRATIONS[..52] {
        conn.execute_batch(migration.sql)?;
    }

    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key)
         VALUES ('test/proj', 'Über Task', 'active',
                 1700000000, 1700000100, 'test/proj', 'test/proj', 'repo', 'test/proj')",
        [],
    )?;
    apply_workstream_identity_migration(&conn)?;

    let normalized_title: String = conn.query_row(
        "SELECT normalized_title FROM workstream_aliases WHERE workstream_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(normalized_title, "über task");

    Ok(())
}
