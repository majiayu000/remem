use anyhow::Result;
use rusqlite::{params, Connection};

use super::*;

fn migrated_connection() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn insert_active_memory(conn: &Connection) -> Result<i64> {
    conn.execute(
        "INSERT INTO memories(
             session_id, project, topic_key, title, content, memory_type,
             created_at_epoch, updated_at_epoch, status
         ) VALUES ('s1', 'project-web', 'web-governance', 'Web memory',
                   'Content', 'decision', 1, 1, 'active')",
        [],
    )?;
    Ok(conn.last_insert_rowid())
}

fn request<'a>(
    memory_id: i64,
    action: WebMemoryGovernanceAction,
    version: i64,
    operation_id: &'a str,
) -> WebMemoryGovernanceRequest<'a> {
    WebMemoryGovernanceRequest {
        memory_id,
        action,
        expected_version: version,
        operation_id,
        reason: "  keep the original normalized reason  ",
        actor: "api",
    }
}

fn applied(decision: WebMemoryGovernanceDecision) -> WebMemoryGovernanceResult {
    match decision {
        WebMemoryGovernanceDecision::Applied(result) => result,
        other => panic!("expected applied decision, got {other:?}"),
    }
}

fn insert_archive_ledger(
    conn: &Connection,
    operation_id: &str,
    result: &WebMemoryGovernanceResult,
) -> Result<()> {
    let response = serde_json::json!({
        "response_schema_version": 1,
        "operation_id": operation_id,
        "audit_id": result.audit_id,
        "memory_id": result.memory_id,
        "action": "archive",
        "before_status": "active",
        "after_status": "archived",
        "version": result.version,
        "occurred_at_epoch": result.occurred_at_epoch,
        "replayed": false,
    })
    .to_string();
    conn.execute(
        "INSERT INTO api_mutation_requests(
             idempotency_key_hash, request_hash, operation_id, resource_kind,
             resource_id, action, response_schema_version, response_json,
             audit_id, created_at_epoch
         ) VALUES (?1, ?2, ?3, 'memory', ?4, 'archive', 1, ?5, ?6, ?7)",
        params![
            format!("hash-{operation_id}"),
            format!("request-{operation_id}"),
            operation_id,
            result.memory_id,
            response,
            result.audit_id,
            result.occurred_at_epoch
        ],
    )?;
    Ok(())
}

#[test]
fn web_archive_sets_marker_versions_state_and_writes_exact_audit() -> Result<()> {
    let conn = migrated_connection()?;
    let memory_id = insert_active_memory(&conn)?;
    let result = applied(govern_memory_for_web_in_transaction(
        &conn,
        &request(
            memory_id,
            WebMemoryGovernanceAction::Archive,
            1,
            "op_archive_domain",
        ),
    )?);

    assert_eq!(result.before_status, "active");
    assert_eq!(result.after_status, "archived");
    assert_eq!(result.version, 2);
    let (status, version, marker): (String, i64, Option<String>) = conn.query_row(
        "SELECT status, version, web_archive_operation_id FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        (status.as_str(), version, marker.as_deref()),
        ("archived", 2, Some("op_archive_domain"))
    );
    let detail: String = conn.query_row(
        "SELECT detail FROM events WHERE id = ?1",
        [result.audit_id],
        |row| row.get(0),
    )?;
    let detail: serde_json::Value = serde_json::from_str(&detail)?;
    assert_eq!(detail["reason"], "  keep the original normalized reason  ");
    assert_eq!(detail["operation_id"], "op_archive_domain");
    assert_eq!(detail["previous_status"], "active");
    assert_eq!(detail["new_status"], "archived");
    Ok(())
}

#[test]
fn web_restore_requires_current_exact_archive_provenance() -> Result<()> {
    let conn = migrated_connection()?;
    let memory_id = insert_active_memory(&conn)?;
    let archive = applied(govern_memory_for_web_in_transaction(
        &conn,
        &request(
            memory_id,
            WebMemoryGovernanceAction::Archive,
            1,
            "op_archive_provenance",
        ),
    )?);
    assert_eq!(
        govern_memory_for_web_in_transaction(
            &conn,
            &request(
                memory_id,
                WebMemoryGovernanceAction::Restore,
                archive.version,
                "op_restore_before_ledger",
            ),
        )?,
        WebMemoryGovernanceDecision::NotRecoverable
    );

    insert_archive_ledger(&conn, "op_archive_provenance", &archive)?;
    let restored = applied(govern_memory_for_web_in_transaction(
        &conn,
        &request(
            memory_id,
            WebMemoryGovernanceAction::Restore,
            archive.version,
            "op_restore_domain",
        ),
    )?);
    assert_eq!(restored.after_status, "active");
    assert_eq!(restored.version, archive.version + 1);
    let marker: Option<String> = conn.query_row(
        "SELECT web_archive_operation_id FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(marker, None);
    Ok(())
}

#[test]
fn historical_ledger_does_not_authorize_non_web_archive() -> Result<()> {
    let conn = migrated_connection()?;
    let memory_id = insert_active_memory(&conn)?;
    let archive = applied(govern_memory_for_web_in_transaction(
        &conn,
        &request(
            memory_id,
            WebMemoryGovernanceAction::Archive,
            1,
            "op_historical_archive",
        ),
    )?);
    insert_archive_ledger(&conn, "op_historical_archive", &archive)?;
    let restored = applied(govern_memory_for_web_in_transaction(
        &conn,
        &request(
            memory_id,
            WebMemoryGovernanceAction::Restore,
            archive.version,
            "op_historical_restore",
        ),
    )?);
    conn.execute(
        "UPDATE memories SET status = 'archived', updated_at_epoch = updated_at_epoch + 1 WHERE id = ?1",
        [memory_id],
    )?;
    let version: i64 = conn.query_row(
        "SELECT version FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(version, restored.version + 1);
    assert_eq!(
        govern_memory_for_web_in_transaction(
            &conn,
            &request(
                memory_id,
                WebMemoryGovernanceAction::Restore,
                version,
                "op_non_web_restore",
            ),
        )?,
        WebMemoryGovernanceDecision::NotRecoverable
    );
    Ok(())
}

#[test]
fn guarded_decisions_do_not_mutate_or_audit() -> Result<()> {
    let conn = migrated_connection()?;
    let memory_id = insert_active_memory(&conn)?;
    assert_eq!(
        govern_memory_for_web_in_transaction(
            &conn,
            &request(
                memory_id,
                WebMemoryGovernanceAction::Archive,
                99,
                "op_stale",
            ),
        )?,
        WebMemoryGovernanceDecision::VersionConflict
    );
    assert_eq!(
        govern_memory_for_web_in_transaction(
            &conn,
            &request(
                memory_id,
                WebMemoryGovernanceAction::Restore,
                1,
                "op_active_restore",
            ),
        )?,
        WebMemoryGovernanceDecision::NotRecoverable
    );
    assert_eq!(
        govern_memory_for_web_in_transaction(
            &conn,
            &request(
                memory_id + 100,
                WebMemoryGovernanceAction::Restore,
                1,
                "op_missing_restore",
            ),
        )?,
        WebMemoryGovernanceDecision::NotRecoverable
    );
    let state: (String, i64) = conn.query_row(
        "SELECT status, version FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state, ("active".to_string(), 1));
    let audits: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(audits, 0);
    Ok(())
}
