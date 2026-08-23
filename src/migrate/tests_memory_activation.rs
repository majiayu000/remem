use anyhow::Result;
use rusqlite::{params, Connection};

use super::run_migrations;

#[test]
fn latest_schema_creates_immutable_activation_ledger_with_result_trust() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;

    assert_eq!(super::latest_schema_version(), 90);
    for object in [
        "memory_activation_requests",
        "idx_memory_activation_result",
        "memory_activation_requests_no_update",
        "memory_activation_requests_no_delete",
        "memory_activation_requests_local_copy_receipt_insert",
        "memory_scope_cleanup_receipts",
        "idx_memory_scope_cleanup_receipts_operation",
        "memory_scope_cleanup_receipts_validate_insert",
        "memory_scope_cleanup_receipts_no_update",
        "memory_scope_cleanup_receipts_no_delete",
        "memory_operation_log_activation_no_replace",
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
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
          claim_id, claim_error, local_copy_status, local_copy_path,
          local_copy_saved_at, local_copy_sha256, created_at_epoch)
         VALUES ('test:receipt', ?1, 'supplemental_save', 'agent', 'save_memory',
                 'external_content', 'local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'supplemental_save',
                 'mcp:test', ?1, ?1, 'clean', '[]', ?2, 'saved', 42, NULL,
                 'saved', '/tmp/remem-note.md', '2026-08-23T00:00:00+00:00', ?1, 1)",
        params!["b".repeat(64), memory_id],
    )?;
    assert!(conn
        .execute(
            "INSERT INTO memory_activation_requests
             (activation_id, request_sha256, route_kind, actor_kind, source_operation,
              source_trust_class, result_source_trust_class, source_project, project,
              branch_present, branch, scope, owner_scope, owner_key, provenance_kind,
              provenance_ref, payload_sha256, result_sha256, poisoning_verdict,
              superseded_ids_json, result_memory_id, claim_status, claim_id,
              claim_error, created_at_epoch)
             VALUES ('test:invalid-receipt', ?1, 'supplemental_save', 'agent',
                     'save_memory', 'external_content', 'local_tool_output', '/repo',
                     '/repo', 0, NULL, 'project', 'repo', '/repo',
                     'supplemental_save', 'mcp:test', ?1, ?1, 'clean', '[]', ?2,
                     'saved', NULL, NULL, 1)",
            params!["c".repeat(64), memory_id],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO memory_activation_requests
             (activation_id, request_sha256, route_kind, actor_kind, source_operation,
              source_trust_class, result_source_trust_class, source_project, project,
              branch_present, branch, scope, owner_scope, owner_key, provenance_kind,
              provenance_ref, payload_sha256, result_sha256, poisoning_verdict,
              superseded_ids_json, result_memory_id, claim_status, claim_id,
              claim_error, created_at_epoch)
             VALUES ('test:zero-claim', ?1, 'supplemental_save', 'agent',
                     'save_memory', 'external_content', 'local_tool_output', '/repo',
                     '/repo', 0, NULL, 'project', 'repo', '/repo',
                     'supplemental_save', 'mcp:test', ?1, ?1, 'clean', '[]', ?2,
                     'saved', 0, NULL, 1)",
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
    assert!(conn
        .execute(
            "INSERT INTO memory_activation_requests
             (activation_id, request_sha256, route_kind, actor_kind, source_operation,
              source_trust_class, result_source_trust_class, source_project, project,
              branch_present, branch, scope, owner_scope, owner_key, target_project,
              provenance_kind, provenance_ref, payload_sha256, result_sha256,
              poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
              local_copy_path, local_copy_saved_at, local_copy_sha256, created_at_epoch)
             VALUES ('test:null-status-local-copy', ?1, 'supplemental_save', 'agent',
                     'save_memory', 'external_content', 'external_content', '/repo',
                     '/repo', 0, NULL, 'project', 'repo', '/repo', '/repo',
                     'supplemental_save', 'mcp:test', ?1, ?1, 'clean', '[]', ?2,
                     'disabled', '/tmp/remem-note.md', '2026-08-23T00:00:00Z',
                     ?1, 1)",
            params!["2".repeat(64), memory_id],
        )
        .is_err());
    Ok(())
}

#[test]
fn v090_scope_cleanup_response_receipt_is_bound_and_immutable() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    run_migrations(&conn)?;
    let memory_id = insert_fixture_memory(&conn)?;
    let stale_memory_id = insert_fixture_memory(&conn)?;
    let superseded_ids = serde_json::to_string(&[stale_memory_id])?;
    conn.execute(
        "INSERT INTO memory_operation_log
         (operation, planner_version, actor, source, result_memory_id,
          superseded_ids, conflicting_ids, reason, created_at_epoch)
         VALUES ('update', 'memory-cleanup-v1', 'memory_cleanup', 'memory_cleanup',
                 ?1, ?2, '[]', 'test cleanup', 1)",
        params![memory_id, superseded_ids],
    )?;
    let operation_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, source_operation_id, created_at_epoch)
         VALUES ('duplicates', ?1, ?2, ?3, 1)",
        params![stale_memory_id, memory_id, operation_id],
    )?;
    let edge_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, created_at_epoch)
         VALUES ('scope:test', ?1, 'scope_cleanup', 'operator', 'memory_cleanup',
                 'local_tool_output', 'local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'scope_plan',
                 'memory-cleanup-v1:1:test', ?1, ?1, 'upstream_validated', ?2, ?3, 1)",
        params!["f".repeat(64), superseded_ids, memory_id],
    )?;
    let owner = || {
        serde_json::json!({
            "source_project": null,
            "target_project": null,
            "owner_scope": null,
            "owner_key": null,
            "topic_domain": null,
            "routing_confidence": null,
            "routing_reason": null,
            "context_class": null,
        })
    };
    let affected = |object_ref: String, new_status: &str| {
        serde_json::json!({
            "object_ref": object_ref,
            "title": "fixture 记忆",
            "previous_status": "active",
            "new_status": new_status,
            "previous_owner": owner(),
            "new_owner": owner(),
        })
    };
    let response = serde_json::json!({
        "current_id": memory_id,
        "stale_ids": [stale_memory_id],
        "operation_id": operation_id,
        "edge_count": 1,
        "affected": [
            affected(format!("memory:{memory_id}"), "active"),
            affected(format!("memory:{stale_memory_id}"), "stale"),
        ],
    })
    .to_string();
    let mismatched_response = serde_json::json!({
        "current_id": memory_id,
        "stale_ids": [],
        "operation_id": operation_id,
        "edge_count": 1,
        "affected": [affected(format!("memory:{memory_id}"), "active")],
    })
    .to_string();
    let duplicate_top_level_key = response.replacen(
        &format!("\"current_id\":{memory_id}"),
        &format!("\"current_id\":{memory_id},\"current_id\":{memory_id}"),
        1,
    );
    let duplicate_owner_key = response.replacen(
        "\"source_project\":null",
        "\"source_project\":null,\"source_project\":null",
        1,
    );
    let invalid_surrogate = response.replacen("\"fixture 记忆\"", r#""\ud800""#, 1);
    let out_of_range_real = response.replacen(
        "\"routing_confidence\":null",
        "\"routing_confidence\":1e400",
        1,
    );
    let string_stale_id = response.replacen(
        &format!("\"stale_ids\":[{stale_memory_id}]"),
        &format!("\"stale_ids\":[\"{stale_memory_id}\"]"),
        1,
    );
    let mut string_affected_value: serde_json::Value = serde_json::from_str(&response)?;
    let encoded_affected = string_affected_value["affected"][0].to_string();
    string_affected_value["affected"][0] = serde_json::Value::String(encoded_affected);
    let string_affected_object = string_affected_value.to_string();
    for invalid_response in [
        serde_json::json!({}).to_string(),
        duplicate_top_level_key,
        duplicate_owner_key,
        invalid_surrogate,
        out_of_range_real,
        string_stale_id,
        string_affected_object,
        serde_json::json!({
            "current_id": memory_id,
            "stale_ids": [stale_memory_id],
            "operation_id": operation_id,
            "edge_count": 1,
            "affected": [
                {"object_ref": format!("memory:{memory_id}")},
                affected(format!("memory:{stale_memory_id}"), "stale"),
            ],
        })
        .to_string(),
        serde_json::json!({
            "current_id": memory_id,
            "stale_ids": [stale_memory_id],
            "operation_id": operation_id,
            "edge_count": 1,
            "affected": [
                {
                    "object_ref": format!("memory:{memory_id}"),
                    "title": "fixture 记忆",
                    "previous_status": "active",
                    "new_status": "active",
                    "previous_owner": {
                        "source_project": 1,
                        "target_project": null,
                        "owner_scope": null,
                        "owner_key": null,
                        "topic_domain": null,
                        "routing_confidence": null,
                        "routing_reason": null,
                        "context_class": null,
                    },
                    "new_owner": owner(),
                },
                affected(format!("memory:{stale_memory_id}"), "stale"),
            ],
        })
        .to_string(),
    ] {
        conn.execute(
            "UPDATE memory_operation_log SET scope_cleanup_response_json = ?1 WHERE id = ?2",
            params![invalid_response, operation_id],
        )?;
        assert!(conn
            .execute(
                "INSERT INTO memory_scope_cleanup_receipts
                 (activation_id, result_memory_id, operation_id, response_json, created_at_epoch)
                 VALUES ('scope:test', ?1, ?2, ?3, 1)",
                params![memory_id, operation_id, invalid_response],
            )
            .is_err());
    }
    conn.execute(
        "UPDATE memory_operation_log SET scope_cleanup_response_json = ?1 WHERE id = ?2",
        params![mismatched_response, operation_id],
    )?;
    assert!(conn
        .execute(
            "INSERT INTO memory_scope_cleanup_receipts
             (activation_id, result_memory_id, operation_id, response_json, created_at_epoch)
             VALUES ('scope:test', ?1, ?2, ?3, 1)",
            params![memory_id, operation_id, mismatched_response],
        )
        .is_err());
    conn.execute(
        "UPDATE memory_operation_log
         SET scope_cleanup_response_json = replace(?1, 'fixture', CAST(X'80' AS TEXT))
         WHERE id = ?2",
        params![response, operation_id],
    )?;
    assert!(conn
        .execute(
            "INSERT INTO memory_scope_cleanup_receipts
             (activation_id, result_memory_id, operation_id, response_json, created_at_epoch)
             SELECT 'scope:test', ?1, ?2, scope_cleanup_response_json, 1
             FROM memory_operation_log WHERE id = ?2",
            params![memory_id, operation_id],
        )
        .is_err());
    conn.execute(
        "UPDATE memory_operation_log SET scope_cleanup_response_json = ?1 WHERE id = ?2",
        params![response, operation_id],
    )?;
    conn.execute(
        "INSERT INTO memory_scope_cleanup_receipts
         (activation_id, result_memory_id, operation_id, response_json, created_at_epoch)
         VALUES ('scope:test', ?1, ?2, ?3, 1)",
        params![memory_id, operation_id, response],
    )?;
    conn.execute(
        "UPDATE memory_operation_log SET activation_id = 'scope:test' WHERE id = ?1",
        [operation_id],
    )?;

    assert!(conn
        .execute(
            "UPDATE memory_scope_cleanup_receipts SET created_at_epoch = 2
             WHERE activation_id = 'scope:test'",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "DELETE FROM memory_scope_cleanup_receipts WHERE activation_id = 'scope:test'",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT OR REPLACE INTO memory_scope_cleanup_receipts
             (activation_id, result_memory_id, operation_id, response_json, created_at_epoch)
             VALUES ('scope:test', ?1, ?2, ?3, 2)",
            params![memory_id, operation_id, response],
        )
        .is_err());
    assert!(conn
        .execute(
            "UPDATE memory_operation_log SET reason = 'changed' WHERE id = ?1",
            [operation_id],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT OR REPLACE INTO memory_operation_log
             (id, operation, planner_version, actor, source, result_memory_id,
              superseded_ids, conflicting_ids, reason, created_at_epoch)
             VALUES (?1, 'update', 'memory-cleanup-v1', 'memory_cleanup',
                     'memory_cleanup', ?2, ?3, '[]', 'replacement', 2)",
            params![operation_id, memory_id, superseded_ids],
        )
        .is_err());
    assert!(conn
        .execute(
            "DELETE FROM memory_operation_log WHERE id = ?1",
            [operation_id]
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO memory_edges
             (edge_type, from_memory_id, to_memory_id, source_operation_id, created_at_epoch)
             VALUES ('duplicates', ?1, ?2, ?3, 2)",
            params![stale_memory_id, memory_id, operation_id],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT OR REPLACE INTO memory_edges
             (id, edge_type, from_memory_id, to_memory_id, source_operation_id,
              created_at_epoch)
             VALUES (?1, 'duplicates', ?2, ?3, NULL, 2)",
            params![edge_id, stale_memory_id, memory_id],
        )
        .is_err());
    assert!(conn
        .execute(
            "UPDATE memory_edges SET reason = 'changed' WHERE id = ?1",
            [edge_id],
        )
        .is_err());
    assert!(conn
        .execute("DELETE FROM memory_edges WHERE id = ?1", [edge_id])
        .is_err());
    Ok(())
}

#[test]
fn v088_upgrades_an_already_applied_v087_database_without_current_state_guessing() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    for migration in &super::MIGRATIONS[..85] {
        conn.execute_batch(migration.sql)?;
    }
    let memory_id = insert_fixture_memory(&conn)?;

    conn.execute_batch(super::MIGRATIONS[85].sql)?;
    insert_v086_activation(&conn, memory_id)?;
    let receipt_rowid_before: i64 = conn.query_row(
        "SELECT rowid FROM memory_activation_requests WHERE activation_id = 'test:v086'",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE memories SET source_trust_class = 'repo_file' WHERE id = ?1",
        [memory_id],
    )?;
    conn.execute_batch(super::MIGRATIONS[86].sql)?;

    let v087_result_trust: String = conn.query_row(
        "SELECT result_source_trust_class FROM memory_activation_requests
         WHERE activation_id = 'test:v086'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(v087_result_trust, "legacy_unrecorded");
    conn.execute_batch(super::MIGRATIONS[87].sql)?;

    let (content, result_trust, receipt_rowid_after): (String, String, i64) = conn.query_row(
        "SELECT memory.content, receipt.result_source_trust_class, receipt.rowid
         FROM memories AS memory
         JOIN memory_activation_requests AS receipt ON receipt.result_memory_id = memory.id
         WHERE memory.id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(content, "existing memory");
    assert_eq!(result_trust, "legacy_v086_source_external_content");
    assert_eq!(receipt_rowid_after, receipt_rowid_before);

    let (not_null, default_value): (i64, Option<String>) = conn.query_row(
        "SELECT \"notnull\", dflt_value
         FROM pragma_table_info('memory_activation_requests')
         WHERE name = 'result_source_trust_class'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(not_null, 1);
    assert_eq!(default_value, None);
    Ok(())
}

#[test]
fn v089_preserves_legacy_receipts_and_rejects_partial_local_copy_evidence() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    for migration in &super::MIGRATIONS[..88] {
        conn.execute_batch(migration.sql)?;
    }
    let memory_id = insert_fixture_memory(&conn)?;
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
          created_at_epoch)
         VALUES ('test:v088-local-copy', ?1, 'supplemental_save', 'agent', 'save_memory',
                 'external_content', 'external_content', '/repo', '/repo', 0, NULL,
                 'project', 'repo', '/repo', '/repo', 'supplemental_save', 'mcp:test',
                 ?1, ?1, 'clean', '[]', ?2, 'disabled', 1)",
        params!["f".repeat(64), memory_id],
    )?;
    let rowid_before: i64 = conn.query_row(
        "SELECT rowid FROM memory_activation_requests WHERE activation_id = 'test:v088-local-copy'",
        [],
        |row| row.get(0),
    )?;

    conn.execute_batch(super::MIGRATIONS[88].sql)?;

    let migrated: (
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT rowid, local_copy_status, local_copy_path,
                    local_copy_saved_at, local_copy_sha256
             FROM memory_activation_requests
             WHERE activation_id = 'test:v088-local-copy'",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(migrated, (rowid_before, None, None, None, None));
    assert!(conn
        .execute(
            "INSERT INTO memory_activation_requests
             (activation_id, request_sha256, route_kind, actor_kind, source_operation,
              source_trust_class, result_source_trust_class, source_project, project,
              branch_present, branch, scope, owner_scope, owner_key, target_project,
              provenance_kind, provenance_ref, payload_sha256, result_sha256,
              poisoning_verdict, superseded_ids_json, result_memory_id, claim_status,
              local_copy_status, local_copy_path, created_at_epoch)
             VALUES ('test:partial-local-copy', ?1, 'supplemental_save', 'agent', 'save_memory',
                     'external_content', 'external_content', '/repo', '/repo', 0, NULL,
                     'project', 'repo', '/repo', '/repo', 'supplemental_save', 'mcp:test',
                     ?1, ?1, 'clean', '[]', ?2, 'disabled', 'saved',
                     '/tmp/remem-note.md', 1)",
            params!["1".repeat(64), memory_id],
        )
        .is_err());
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
          source_trust_class, result_source_trust_class, source_project, project,
          branch_present, branch, scope, owner_scope, owner_key, target_project,
          provenance_kind, provenance_ref, payload_sha256, result_sha256,
          poisoning_verdict, superseded_ids_json, result_memory_id, created_at_epoch)
         VALUES ('test:1', ?1, 'rust_api', 'rust_api', 'test',
                 'local_tool_output', 'local_tool_output', '/repo', '/repo', 0,
                 NULL, 'project', 'repo', '/repo', '/repo', 'rust_api', 'test:v1',
                 ?1, ?1, 'clean', '[]', ?2, 1)",
        params!["a".repeat(64), memory_id],
    )?;
    Ok(())
}

fn insert_v086_activation(conn: &Connection, memory_id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_activation_requests
         (activation_id, request_sha256, route_kind, actor_kind, source_operation,
          source_trust_class, source_project, project, branch_present, branch, scope,
          owner_scope, owner_key, target_project, provenance_kind, provenance_ref,
          payload_sha256, result_sha256, poisoning_verdict, superseded_ids_json,
          result_memory_id, created_at_epoch)
         VALUES ('test:v086', ?1, 'rust_api', 'rust_api', 'test',
                 'external_content', '/repo', '/repo', 0, NULL, 'project', 'repo',
                 '/repo', '/repo', 'rust_api', 'test:v086', ?1, ?1, 'clean', '[]', ?2, 1)",
        params!["e".repeat(64), memory_id],
    )?;
    Ok(())
}
