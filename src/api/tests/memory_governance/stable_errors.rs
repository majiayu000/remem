use axum::http::StatusCode;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::json;

use crate::api::mutation::{
    mutation_request_hash, validate_idempotency_key, CredentialFreeMutationBody,
};
use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::{insert_memory, memory_version, response_json, send_governance};
use crate::api::handlers::execute_memory_governance_for_test;

#[tokio::test]
async fn validation_and_state_errors_are_stable_and_side_effect_free() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-errors");
    let memory_id = insert_memory("memory-governance-errors")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let version = memory_version(memory_id)?;

    for (body, code, has_operation_id) in [
        (
            json!({
                "reason": "valid",
                "expected_version": version,
                "idempotency_key": "contains spaces"
            }),
            "idempotency_key_invalid",
            false,
        ),
        (
            json!({
                "reason": "   ",
                "expected_version": version,
                "idempotency_key": "empty-reason-key"
            }),
            "reason_invalid",
            true,
        ),
        (
            json!({
                "reason": "valid",
                "expected_version": 0,
                "idempotency_key": "bad-version-key"
            }),
            "memory_governance_request_invalid",
            true,
        ),
    ] {
        let response = send_governance(memory_id, "archive", &token, body).await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await?;
        assert_eq!(payload["error"]["code"], code);
        assert_eq!(
            payload["error"]["operation_id"].is_string(),
            has_operation_id
        );
    }

    let stale = send_governance(
        memory_id,
        "archive",
        &token,
        json!({
            "reason": "stale version",
            "expected_version": version + 1,
            "idempotency_key": "stale-memory-version-key"
        }),
    )
    .await?;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(stale).await?["error"]["code"],
        "version_conflict"
    );

    let restore_active = send_governance(
        memory_id,
        "restore",
        &token,
        json!({
            "reason": "not Web archived",
            "expected_version": version,
            "idempotency_key": "restore-active-key"
        }),
    )
    .await?;
    assert_eq!(restore_active.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(restore_active).await?["error"]["code"],
        "memory_not_recoverable"
    );

    let missing = send_governance(
        9_999_999,
        "restore",
        &token,
        json!({
            "reason": "missing memory",
            "expected_version": 1,
            "idempotency_key": "restore-missing-key"
        }),
    )
    .await?;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(missing).await?["error"]["code"],
        "memory_not_recoverable"
    );

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memories SET status = 'archived', updated_at_epoch = updated_at_epoch + 1
         WHERE id = ?1",
        [memory_id],
    )?;
    drop(conn);
    let nonactive = send_governance(
        memory_id,
        "archive",
        &token,
        json!({
            "reason": "cannot archive twice",
            "expected_version": memory_version(memory_id)?,
            "idempotency_key": "archive-nonactive-key"
        }),
    )
    .await?;
    assert_eq!(nonactive.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(nonactive).await?["error"]["code"],
        "memory_not_archivable"
    );

    let conn = db::open_db()?;
    let (status, audits, ledgers): (String, i64, i64) = conn.query_row(
        "SELECT status,
                (SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'),
                (SELECT COUNT(*) FROM api_mutation_requests)
         FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!((status.as_str(), audits, ledgers), ("archived", 0, 0));
    Ok(())
}

#[test]
fn ledger_failure_rolls_back_state_marker_and_audit() -> anyhow::Result<()> {
    let mut conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO memories(project, title, content, memory_type,
                              created_at_epoch, updated_at_epoch, status)
         VALUES ('p', 'rollback', 'content', 'decision', 1, 1, 'active')",
        [],
    )?;
    let memory_id = conn.last_insert_rowid();
    conn.execute_batch(
        "CREATE TRIGGER reject_memory_governance_ledger
         BEFORE INSERT ON api_mutation_requests
         BEGIN SELECT RAISE(ABORT, 'forced ledger failure'); END;",
    )?;
    let response = execute_memory_governance_for_test(
        &mut conn,
        memory_id,
        "archive",
        serde_json::to_string(&json!({
            "reason": "must roll back",
            "expected_version": 1,
            "idempotency_key": "memory-ledger-rollback"
        }))?
        .as_bytes(),
    );
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    assert_eq!(
        runtime.block_on(response_json(response))?["error"]["code"],
        "memory_governance_ledger_failed"
    );
    let (status, version, marker, audits, ledgers): (String, i64, Option<String>, i64, i64) = conn
        .query_row(
            "SELECT status, version, web_archive_operation_id,
                (SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'),
                (SELECT COUNT(*) FROM api_mutation_requests)
         FROM memories WHERE id = ?1",
            [memory_id],
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
    assert_eq!(
        (status.as_str(), version, marker, audits, ledgers),
        ("active", 1, None, 0, 0)
    );
    Ok(())
}

#[derive(Serialize)]
struct ArchiveHashFixture<'a> {
    reason: &'a str,
    expected_version: i64,
}

impl CredentialFreeMutationBody for ArchiveHashFixture<'_> {}

#[tokio::test]
async fn unknown_replay_schema_has_a_stable_conflict() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-schema");
    let memory_id = insert_memory("memory-governance-schema")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let version = memory_version(memory_id)?;
    let identity = validate_idempotency_key("memory-unknown-schema")?;
    let request_hash = mutation_request_hash(
        "memory",
        memory_id,
        "archive",
        &ArchiveHashFixture {
            reason: "schema replay",
            expected_version: version,
        },
    )?;
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO api_mutation_requests(
             idempotency_key_hash, request_hash, operation_id, resource_kind,
             resource_id, action, response_schema_version, response_json,
             audit_id, created_at_epoch)
         VALUES (?1, ?2, ?3, 'memory', ?4, 'archive', 99, '{}', 1, 1)",
        params![
            identity.idempotency_key_hash,
            request_hash,
            identity.operation_id,
            memory_id
        ],
    )?;
    drop(conn);
    let response = send_governance(
        memory_id,
        "archive",
        &token,
        json!({
            "reason": "schema replay",
            "expected_version": version,
            "idempotency_key": "memory-unknown-schema"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "idempotency_schema_unsupported"
    );
    Ok(())
}

#[tokio::test]
async fn non_web_archive_sequence_remains_nonrecoverable() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-non-web-sequence");
    let memory_id = insert_memory("memory-governance-non-web-sequence")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let first_version = memory_version(memory_id)?;
    let archived = send_governance(
        memory_id,
        "archive",
        &token,
        json!({
            "reason": "first Web archive",
            "expected_version": first_version,
            "idempotency_key": "sequence-web-archive"
        }),
    )
    .await?;
    let archived = response_json(archived).await?;
    let restored = send_governance(
        memory_id,
        "restore",
        &token,
        json!({
            "reason": "first Web restore",
            "expected_version": archived["version"],
            "idempotency_key": "sequence-web-restore"
        }),
    )
    .await?;
    assert_eq!(restored.status(), StatusCode::OK);
    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memories SET status = 'archived', updated_at_epoch = updated_at_epoch + 1
         WHERE id = ?1",
        [memory_id],
    )?;
    drop(conn);
    let version = memory_version(memory_id)?;
    let response = send_governance(
        memory_id,
        "restore",
        &token,
        json!({
            "reason": "historical ledger is insufficient",
            "expected_version": version,
            "idempotency_key": "sequence-non-web-restore"
        }),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(response).await?["error"]["code"],
        "memory_not_recoverable"
    );

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memories SET status = 'active', updated_at_epoch = updated_at_epoch + 1
         WHERE id = ?1",
        [memory_id],
    )?;
    drop(conn);
    let fresh_archive = send_governance(
        memory_id,
        "archive",
        &token,
        json!({
            "reason": "fresh Web archive creates current provenance",
            "expected_version": memory_version(memory_id)?,
            "idempotency_key": "sequence-fresh-web-archive"
        }),
    )
    .await?;
    assert_eq!(fresh_archive.status(), StatusCode::OK);
    let fresh_archive = response_json(fresh_archive).await?;
    let fresh_restore = send_governance(
        memory_id,
        "restore",
        &token,
        json!({
            "reason": "fresh provenance restores",
            "expected_version": fresh_archive["version"],
            "idempotency_key": "sequence-fresh-web-restore"
        }),
    )
    .await?;
    assert_eq!(fresh_restore.status(), StatusCode::OK);
    assert_eq!(
        response_json(fresh_restore).await?["after_status"],
        "active"
    );
    Ok(())
}
