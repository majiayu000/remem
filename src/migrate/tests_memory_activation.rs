use anyhow::Result;
use rusqlite::{params, Connection};

use super::run_migrations;

#[test]
fn v086_creates_immutable_activation_ledger() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    assert_eq!(super::latest_schema_version(), 86);
    for object in [
        "memory_activation_requests",
        "idx_memory_activation_result",
        "memory_activation_requests_no_update",
        "memory_activation_requests_no_delete",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
            [object],
            |row| row.get(0),
        )?;
        assert!(exists, "missing v086 object {object}");
    }

    let memory_id = insert_fixture_memory(&conn)?;
    insert_activation(&conn, memory_id)?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, source_project, project, branch_present, branch, scope, owner_scope,
          owner_key, target_project, provenance_kind, provenance_ref, payload_sha256,
          result_sha256, poisoning_verdict, superseded_ids_json, result_memory_id,
          claim_status, claim_id, claim_error, created_at_epoch)
         VALUES ('test:receipt', ?1, 'supplemental_save', 'agent', 'save_memory',
                 'external_content', '/repo', '/repo', 0, NULL, 'project', 'repo',
                 '/repo', '/repo', 'supplemental_save', 'mcp:test', ?1, ?1,
                 'clean', '[]', ?2, 'saved', 42, NULL, 1)",
        params!["b".repeat(64), memory_id],
    )?;
    assert!(conn
        .execute(
            "INSERT INTO memory_activation_requests
             (activation_id, request_sha256, route_kind, actor_kind, source_operation,
              source_trust_class, source_project, project, branch_present, branch, scope,
              owner_scope, owner_key, provenance_kind, provenance_ref, payload_sha256,
              result_sha256, poisoning_verdict, superseded_ids_json, result_memory_id,
              claim_status, claim_id, claim_error, created_at_epoch)
             VALUES ('test:invalid-receipt', ?1, 'supplemental_save', 'agent',
                     'save_memory', 'external_content', '/repo', '/repo', 0, NULL,
                     'project', 'repo', '/repo', 'supplemental_save', 'mcp:test',
                     ?1, ?1, 'clean', '[]', ?2, 'saved', NULL, NULL, 1)",
            params!["c".repeat(64), memory_id],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO memory_activation_requests
             (activation_id, request_sha256, route_kind, actor_kind, source_operation,
              source_trust_class, source_project, project, branch_present, branch, scope,
              owner_scope, owner_key, provenance_kind, provenance_ref, payload_sha256,
              result_sha256, poisoning_verdict, superseded_ids_json, result_memory_id,
              claim_status, claim_id, claim_error, created_at_epoch)
             VALUES ('test:zero-claim', ?1, 'supplemental_save', 'agent',
                     'save_memory', 'external_content', '/repo', '/repo', 0, NULL,
                     'project', 'repo', '/repo', 'supplemental_save', 'mcp:test',
                     ?1, ?1, 'clean', '[]', ?2, 'saved', 0, NULL, 1)",
            params!["d".repeat(64), memory_id],
        )
        .is_err());
    assert!(conn
        .execute(
            "UPDATE memory_activation_requests SET source_operation = 'changed' WHERE activation_id = 'test:1'",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "DELETE FROM memory_activation_requests WHERE activation_id = 'test:1'",
            [],
        )
        .is_err());
    Ok(())
}

#[test]
fn v086_upgrades_a_v085_database_without_rewriting_memories() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    for migration in &super::MIGRATIONS[..85] {
        conn.execute_batch(migration.sql)?;
    }
    let memory_id = insert_fixture_memory(&conn)?;

    conn.execute_batch(super::MIGRATIONS[85].sql)?;

    let content: String = conn.query_row(
        "SELECT content FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(content, "existing memory");
    insert_activation(&conn, memory_id)?;
    Ok(())
}

fn insert_fixture_memory(conn: &Connection) -> Result<i64> {
    conn.execute(
        "INSERT INTO memories
         (project, title, content, memory_type, created_at_epoch, updated_at_epoch,
          status, scope, source_trust_class)
         VALUES ('/repo', 'existing', 'existing memory', 'discovery', 1, 1,
                 'active', 'project', 'local_tool_output')",
        [],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_activation(conn: &Connection, memory_id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, source_project, project, branch_present, branch, scope, owner_scope,
          owner_key, target_project, provenance_kind, provenance_ref, payload_sha256,
          result_sha256, poisoning_verdict, superseded_ids_json, result_memory_id,
          created_at_epoch)
         VALUES ('test:1', ?1, 'rust_api', 'rust_api', 'test',
                 'local_tool_output', '/repo', '/repo', 0, NULL, 'project', 'repo',
                 '/repo', '/repo', 'rust_api', 'test:v1', ?1, ?1, 'clean', '[]', ?2, 1)",
        params!["a".repeat(64), memory_id],
    )?;
    Ok(())
}
