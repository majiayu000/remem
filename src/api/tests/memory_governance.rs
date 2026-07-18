use std::sync::{Arc, Barrier};

use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use rusqlite::params;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::api::mutation::validate_idempotency_key;
use crate::api::DbState;
use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::super::handlers::execute_memory_governance_for_test;
use super::authorized_json_request;

mod stable_errors;

fn insert_memory(fixture: &str) -> anyhow::Result<i64> {
    let conn = db::open_db()?;
    crate::memory::insert_memory(
        &conn,
        Some(fixture),
        "project-web-governance",
        Some(fixture),
        "Web governance memory",
        "A searchable Web governance sentinel.",
        "decision",
        None,
    )
}

fn memory_version(id: i64) -> anyhow::Result<i64> {
    let conn = db::open_db()?;
    conn.query_row("SELECT version FROM memories WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .map_err(Into::into)
}

async fn response_json(response: axum::response::Response) -> anyhow::Result<Value> {
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn send_governance(
    memory_id: i64,
    action: &str,
    token: &str,
    body: Value,
) -> anyhow::Result<axum::response::Response> {
    let app = super::super::build_router(0).with_state(DbState);
    app.oneshot(authorized_json_request(
        Method::POST,
        &format!("/api/v1/memories/{memory_id}/{action}"),
        token,
        &serde_json::to_string(&body)?,
    ))
    .await
    .map_err(Into::into)
}

async fn get_json(uri: &str, token: &str) -> anyhow::Result<(StatusCode, Value)> {
    let app = super::super::build_router(0).with_state(DbState);
    let response = app
        .oneshot(authorized_json_request(Method::GET, uri, token, ""))
        .await?;
    let status = response.status();
    Ok((status, response_json(response).await?))
}

#[tokio::test]
async fn archive_and_restore_are_atomic_audited_and_replayable() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-roundtrip");
    let memory_id = insert_memory("memory-governance-roundtrip")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let initial_version = memory_version(memory_id)?;
    let archive_body = json!({
        "reason": "  remove stale console memory  ",
        "expected_version": initial_version,
        "idempotency_key": "memory-archive-roundtrip-1"
    });
    let response = send_governance(memory_id, "archive", &token, archive_body.clone()).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let archived = response_json(response).await?;
    assert_eq!(archived["action"], "archive");
    assert_eq!(archived["before_status"], "active");
    assert_eq!(archived["after_status"], "archived");
    assert_eq!(archived["replayed"], false);
    assert_eq!(archived["version"], initial_version + 1);

    let conn = db::open_db()?;
    let (status, marker, detail, ledger_count): (String, Option<String>, String, i64) = conn
        .query_row(
            "SELECT m.status, m.web_archive_operation_id, e.detail,
                    (SELECT COUNT(*) FROM api_mutation_requests)
             FROM memories m JOIN events e ON e.id = ?1 WHERE m.id = ?2",
            params![archived["audit_id"].as_i64(), memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    assert_eq!(status, "archived");
    assert_eq!(marker.as_deref(), archived["operation_id"].as_str());
    let detail: Value = serde_json::from_str(&detail)?;
    assert_eq!(detail["reason"], "remove stale console memory");
    assert_eq!(detail["operation_id"], archived["operation_id"]);
    assert_eq!(ledger_count, 1);
    let persisted: String = conn.query_row(
        "SELECT response_json || idempotency_key_hash || request_hash
         FROM api_mutation_requests LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert!(!persisted.contains("memory-archive-roundtrip-1"));
    let stored_key_hash: String = conn.query_row(
        "SELECT idempotency_key_hash FROM api_mutation_requests LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        stored_key_hash,
        validate_idempotency_key("memory-archive-roundtrip-1")?.idempotency_key_hash
    );
    assert!(!serde_json::to_string(&archived)?.contains("memory-archive-roundtrip-1"));
    assert!(!serde_json::to_string(&detail)?.contains("memory-archive-roundtrip-1"));
    drop(conn);

    let replay = send_governance(memory_id, "archive", &token, archive_body).await?;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = response_json(replay).await?;
    assert_eq!(replay["replayed"], true);
    for field in ["operation_id", "audit_id", "memory_id", "version"] {
        assert_eq!(replay[field], archived[field]);
    }

    let restore_body = json!({
        "reason": "reviewed and recoverable",
        "expected_version": archived["version"],
        "idempotency_key": "memory-restore-roundtrip-1"
    });
    let restored = send_governance(memory_id, "restore", &token, restore_body.clone()).await?;
    assert_eq!(restored.status(), StatusCode::OK);
    let restored = response_json(restored).await?;
    assert_eq!(restored["before_status"], "archived");
    assert_eq!(restored["after_status"], "active");
    assert_eq!(restored["version"], initial_version + 2);
    let conn = db::open_db()?;
    let (status, marker, audits, ledgers): (String, Option<String>, i64, i64) = conn.query_row(
        "SELECT status, web_archive_operation_id,
                (SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'),
                (SELECT COUNT(*) FROM api_mutation_requests)
         FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(status, "active");
    assert_eq!(marker, None);
    assert_eq!((audits, ledgers), (2, 2));
    drop(conn);

    let replay = send_governance(memory_id, "restore", &token, restore_body).await?;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = response_json(replay).await?;
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["operation_id"], restored["operation_id"]);
    assert_eq!(replay["audit_id"], restored["audit_id"]);
    Ok(())
}

#[tokio::test]
async fn replay_and_conflict_precede_current_memory_state() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-precedence");
    let memory_id = insert_memory("memory-governance-precedence")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let version = memory_version(memory_id)?;
    let body = json!({
        "reason": "archive once",
        "expected_version": version,
        "idempotency_key": "memory-precedence-key"
    });
    let first = send_governance(memory_id, "archive", &token, body.clone()).await?;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await?;

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memories SET status = 'deleted', updated_at_epoch = updated_at_epoch + 1
         WHERE id = ?1",
        [memory_id],
    )?;
    drop(conn);
    let replay = send_governance(memory_id, "archive", &token, body).await?;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = response_json(replay).await?;
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["operation_id"], first["operation_id"]);

    let conflict = send_governance(
        memory_id,
        "restore",
        &token,
        json!({
            "reason": "different action and payload",
            "expected_version": first["version"],
            "idempotency_key": "memory-precedence-key"
        }),
    )
    .await?;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(conflict).await?["error"]["code"],
        "idempotency_conflict"
    );
    Ok(())
}

#[tokio::test]
async fn list_detail_and_search_observe_archive_state_and_version() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-read-surfaces");
    let memory_id = insert_memory("memory-governance-read-surfaces")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let (_, before_list) = get_json("/api/v1/memories", &token).await?;
    let before_item = before_list["data"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == memory_id))
        .expect("memory in canonical list");
    let version = before_item["version"].as_i64().expect("list version");
    assert_eq!(before_item["status"], "active");
    let (_, detail) = get_json(&format!("/api/v1/memories/{memory_id}"), &token).await?;
    assert_eq!(detail["version"], version);

    let archived = send_governance(
        memory_id,
        "archive",
        &token,
        json!({
            "reason": "hide from active retrieval",
            "expected_version": version,
            "idempotency_key": "memory-read-surfaces-archive"
        }),
    )
    .await?;
    assert_eq!(archived.status(), StatusCode::OK);
    let archived = response_json(archived).await?;

    let (_, canonical) = get_json("/api/v1/memories", &token).await?;
    let canonical_item = canonical["data"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == memory_id))
        .expect("canonical no-status behavior remains inclusive");
    assert_eq!(canonical_item["status"], "archived");
    assert_eq!(canonical_item["version"], archived["version"]);
    let (_, archived_list) = get_json("/api/v1/memories?status=archived", &token).await?;
    assert!(archived_list["data"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == memory_id)));
    let (_, active_list) = get_json("/api/v1/memories?status=active", &token).await?;
    assert!(!active_list["data"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == memory_id)));
    let (_, detail) = get_json(&format!("/api/v1/memories/{memory_id}"), &token).await?;
    assert_eq!(detail["status"], "archived");
    assert_eq!(detail["version"], archived["version"]);
    let (_, search) = get_json("/api/v1/search?q=searchable%20Web%20governance", &token).await?;
    assert!(!search["data"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == memory_id)));
    Ok(())
}

#[tokio::test]
async fn staged_contract_is_disabled_routes_are_authenticated_and_delete_is_absent(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-contract");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);
    let capabilities = app
        .clone()
        .oneshot(authorized_json_request(
            Method::GET,
            "/api/v1/capabilities",
            &token,
            "",
        ))
        .await?;
    assert_eq!(capabilities.status(), StatusCode::OK);
    let capabilities = response_json(capabilities).await?;
    for feature in ["memory_archive", "memory_restore", "memory_delete"] {
        assert_eq!(capabilities["features"][feature], false);
        assert!(capabilities["endpoints"].get(feature).is_none());
    }
    for action in ["archive", "restore"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/v1/memories/1/{action}"))
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let delete = app
        .oneshot(authorized_json_request(
            Method::DELETE,
            "/api/v1/memories/1",
            &token,
            "",
        ))
        .await?;
    assert_eq!(delete.status(), StatusCode::METHOD_NOT_ALLOWED);
    Ok(())
}

#[test]
fn concurrent_same_key_archive_applies_once_and_replays_once() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-memory-governance-concurrent");
    let memory_id = insert_memory("memory-governance-concurrent")?;
    let body = json!({
        "reason": "same concurrent archive",
        "expected_version": memory_version(memory_id)?,
        "idempotency_key": "memory-concurrent-archive-1"
    });
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for mut conn in [db::open_db()?, db::open_db()?] {
        let barrier = Arc::clone(&barrier);
        let body = serde_json::to_vec(&body)?;
        workers.push(std::thread::spawn(move || -> anyhow::Result<Value> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            barrier.wait();
            let response =
                execute_memory_governance_for_test(&mut conn, memory_id, "archive", &body);
            runtime.block_on(async {
                anyhow::ensure!(response.status() == StatusCode::OK, "request failed");
                response_json(response).await
            })
        }));
    }
    let mut payloads = workers
        .into_iter()
        .map(|worker| {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("concurrent archive worker panicked"))?
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    payloads.sort_by_key(|payload| payload["replayed"].as_bool());
    assert_eq!(payloads[0]["replayed"], false);
    assert_eq!(payloads[1]["replayed"], true);
    assert_eq!(payloads[0]["operation_id"], payloads[1]["operation_id"]);
    assert_eq!(payloads[0]["audit_id"], payloads[1]["audit_id"]);
    let conn = db::open_db()?;
    let counts: (i64, i64) = conn.query_row(
        "SELECT
             (SELECT COUNT(*) FROM events WHERE event_type = 'memory_governance'),
             (SELECT COUNT(*) FROM api_mutation_requests)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(counts, (1, 1));
    Ok(())
}
