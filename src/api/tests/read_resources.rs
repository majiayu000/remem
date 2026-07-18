use axum::{
    body::{to_bytes, Body},
    http::{Method, StatusCode},
    Router,
};
use rusqlite::{params, Connection};
use serde_json::Value;
use tower::ServiceExt;

use crate::db;
use crate::db::test_support::ScopedTestDataDir;

use super::{authorized_request, DbState};

const RAW_SENTINEL: &str = "RAW_TRANSCRIPT_SENTINEL_DO_NOT_EXPOSE";
const SECRET_SENTINEL: &str = "token=web-resource-super-secret";

struct Fixture {
    observation_id: i64,
    session_id: i64,
    workstream_id: i64,
    event_id: i64,
    task_id: i64,
}

#[tokio::test]
async fn read_resource_routes_are_authenticated_and_staged_capabilities_are_absent(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-auth");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    for route in list_and_detail_routes(1) {
        let missing = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(Method::GET)
                    .uri(&route)
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED, "{route}");
    }

    let (_, capabilities) = get_json(&app, "/api/v1/capabilities", &token).await?;
    for resource in ["observations", "sessions", "workstreams", "events", "tasks"] {
        assert_eq!(capabilities["features"][resource], false, "{resource}");
        assert!(capabilities["endpoints"]
            .get(format!("{resource}_list"))
            .is_none());
        assert!(capabilities["endpoints"]
            .get(format!("{resource}_detail"))
            .is_none());
    }
    Ok(())
}

#[tokio::test]
async fn every_resource_returns_real_safe_list_and_detail_projection() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-safe-projection");
    let fixture = insert_fixture("safe-projection")?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    for (route, id) in [
        ("/api/v1/observations", fixture.observation_id),
        ("/api/v1/sessions", fixture.session_id),
        ("/api/v1/workstreams", fixture.workstream_id),
        ("/api/v1/events", fixture.event_id),
        ("/api/v1/tasks", fixture.task_id),
    ] {
        let (status, list) = get_json(&app, route, &token).await?;
        assert_eq!(status, StatusCode::OK, "{route}");
        assert!(list["data"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["id"] == id)));
        assert_safe_json(&list);

        let detail_route = format!("{route}/{id}");
        let (status, detail) = get_json(&app, &detail_route, &token).await?;
        assert_eq!(status, StatusCode::OK, "{detail_route}");
        assert_eq!(detail["data"]["id"], id);
        assert_safe_json(&detail);
    }
    Ok(())
}

#[tokio::test]
async fn empty_not_found_and_invalid_id_states_are_distinct() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-empty");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    for route in list_routes() {
        let (status, payload) = get_json(&app, route, &token).await?;
        assert_eq!(status, StatusCode::OK, "{route}");
        assert_eq!(payload["data"], serde_json::json!([]));
        assert!(payload["next_cursor"].is_null());

        let (status, payload) = get_json(&app, &format!("{route}/999999"), &token).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{route}");
        assert_eq!(payload["error"]["code"], "not_found");

        let (status, payload) = get_json(&app, &format!("{route}/not-an-id"), &token).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{route}");
        assert_eq!(payload["error"]["code"], "id_invalid");
    }
    Ok(())
}

#[tokio::test]
async fn cursor_repeat_filter_binding_and_page_size_contract_are_stable() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-cursor");
    let conn = db::open_db()?;
    insert_workstreams(&conn, "cursor", 3)?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    let first_uri = "/api/v1/workstreams?page_size=2&project=cursor-project";
    let (_, first) = get_json(&app, first_uri, &token).await?;
    let (_, repeated) = get_json(&app, first_uri, &token).await?;
    assert_eq!(first, repeated);
    assert_eq!(first["page_size"], 2);
    assert_eq!(first["data"].as_array().map(Vec::len), Some(2));
    let cursor = first["next_cursor"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("first page should continue"))?;
    let first_ids = ids(&first);

    let (_, second) = get_json(
        &app,
        &format!("/api/v1/workstreams?page_size=2&project=cursor-project&cursor={cursor}"),
        &token,
    )
    .await?;
    assert!(ids(&second).iter().all(|id| !first_ids.contains(id)));
    assert_eq!(second["data"].as_array().map(Vec::len), Some(1));
    assert!(second["next_cursor"].is_null());

    let (status, payload) = get_json(
        &app,
        &format!("/api/v1/events?page_size=2&project=cursor-project&cursor={cursor}"),
        &token,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"]["code"], "cursor_invalid");

    let (status, payload) = get_json(
        &app,
        &format!("/api/v1/workstreams?page_size=2&project=other&cursor={cursor}"),
        &token,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"]["code"], "cursor_invalid");

    for invalid in ["nope", "9223372036854775808"] {
        let (status, payload) = get_json(
            &app,
            &format!("/api/v1/workstreams?page_size={invalid}"),
            &token,
        )
        .await?;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(payload["error"]["code"], "page_size_invalid");
    }
    let (_, clamped_low) = get_json(&app, "/api/v1/workstreams?page_size=0", &token).await?;
    assert_eq!(clamped_low["page_size"], 1);
    assert!(clamped_low["data"]
        .as_array()
        .is_some_and(|rows| rows.len() <= 1));
    let (_, clamped_high) = get_json(&app, "/api/v1/workstreams?page_size=101", &token).await?;
    assert_eq!(clamped_high["page_size"], 100);
    assert!(clamped_high["data"]
        .as_array()
        .is_some_and(|rows| rows.len() <= 100));
    Ok(())
}

#[tokio::test]
async fn pattern_and_topic_suppression_hide_rows_then_revocation_restores_them(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-suppression");
    let conn = db::open_db()?;
    let workstream_ids = insert_workstreams(&conn, "suppression", 2)?;
    conn.execute(
        "UPDATE workstreams SET title = 'hidden-pattern title', topic_domain = 'hidden-topic'
         WHERE id = ?1",
        params![workstream_ids[0]],
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('pattern', 'hidden-pattern', 'test', 'test', 'active', ?1, ?1)",
        params![now],
    )?;
    let suppression_id = conn.last_insert_rowid();
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    let (_, list) = get_json(&app, "/api/v1/workstreams", &token).await?;
    assert!(!ids(&list).contains(&workstream_ids[0]));
    let (_, attempted_bypass) =
        get_json(&app, "/api/v1/workstreams?include_suppressed=true", &token).await?;
    assert!(!ids(&attempted_bypass).contains(&workstream_ids[0]));
    let (status, _) = get_json(
        &app,
        &format!("/api/v1/workstreams/{}", workstream_ids[0]),
        &token,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_suppressions SET status = 'revoked' WHERE id = ?1",
        params![suppression_id],
    )?;
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('topic_key', 'hidden-topic', 'test', 'test', 'active', ?1, ?1)",
        params![now],
    )?;
    let topic_suppression_id = conn.last_insert_rowid();
    drop(conn);
    let (_, list) = get_json(&app, "/api/v1/workstreams", &token).await?;
    assert!(!ids(&list).contains(&workstream_ids[0]));

    let conn = db::open_db()?;
    conn.execute(
        "UPDATE memory_suppressions SET status = 'revoked' WHERE id = ?1",
        params![topic_suppression_id],
    )?;
    drop(conn);
    let (_, restored) = get_json(&app, "/api/v1/workstreams", &token).await?;
    assert!(ids(&restored).contains(&workstream_ids[0]));
    Ok(())
}

#[tokio::test]
async fn policy_database_failure_is_a_structured_server_error() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-policy-failure");
    let conn = db::open_db()?;
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('unsupported_policy_kind', 'x', 'test', 'test', 'active', ?1, ?1)",
        params![now],
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);
    let (status, payload) = get_json(&app, "/api/v1/events", &token).await?;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(payload["error"]["code"], "resource_policy_failed");
    assert!(!payload.to_string().contains("memory_suppressions"));
    Ok(())
}

#[tokio::test]
async fn all_suppressed_budget_pages_advance_then_terminate() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-budget");
    let conn = db::open_db()?;
    insert_workstreams(&conn, "budget-blocked", 101)?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_suppressions
         (target_kind, target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
         VALUES ('pattern', 'budget-blocked', 'test', 'test', 'active', ?1, ?1)",
        params![now],
    )?;
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    let (_, first) = get_json(&app, "/api/v1/workstreams?page_size=1", &token).await?;
    assert_eq!(first["data"], serde_json::json!([]));
    let cursor = first["next_cursor"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("budget page should continue"))?;
    let (_, terminal) = get_json(
        &app,
        &format!("/api/v1/workstreams?page_size=1&cursor={cursor}"),
        &token,
    )
    .await?;
    assert_eq!(terminal["data"], serde_json::json!([]));
    assert!(terminal["next_cursor"].is_null());
    Ok(())
}

#[tokio::test]
async fn all_five_cursors_ignore_reinserted_ids_above_the_old_boundary() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-read-resource-reinsert");
    let base = insert_fixture("reinsert-base")?;
    let conn = db::open_db()?;
    let (host_id, workspace_id, project_id, session_row_id, raw_session_id): (
        i64,
        i64,
        i64,
        i64,
        String,
    ) = conn.query_row(
        "SELECT host_id, workspace_id, project_id, id, session_id FROM sessions WHERE id = ?1",
        params![base.session_id],
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
    for kind in ["observations", "sessions", "workstreams", "events", "tasks"] {
        insert_cursor_rows(
            &conn,
            kind,
            "before",
            3,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            &raw_session_id,
        )?;
    }
    drop(conn);
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);

    for (kind, route) in [
        ("observations", "/api/v1/observations"),
        ("sessions", "/api/v1/sessions"),
        ("workstreams", "/api/v1/workstreams"),
        ("events", "/api/v1/events"),
        ("tasks", "/api/v1/tasks"),
    ] {
        let (_, first) = get_json(&app, &format!("{route}?page_size=1"), &token).await?;
        let old_max = ids(&first)[0];
        let cursor = first["next_cursor"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("{kind} should continue"))?
            .to_string();
        let conn = db::open_db()?;
        delete_cursor_row(&conn, kind, old_max)?;
        let new_ids = insert_cursor_rows(
            &conn,
            kind,
            "after",
            1,
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            &raw_session_id,
        )?;
        assert!(new_ids[0] > old_max, "{kind}");
        if kind == "sessions" {
            conn.execute(
                "UPDATE sessions SET last_seen_at_epoch = 9999999999 WHERE id < ?1",
                params![old_max],
            )?;
        }
        drop(conn);
        let (_, next) = get_json(
            &app,
            &format!("{route}?page_size=1&cursor={cursor}"),
            &token,
        )
        .await?;
        assert!(!ids(&next).contains(&new_ids[0]), "{kind}");
        assert!(ids(&next).iter().all(|id| *id < old_max), "{kind}");
    }
    Ok(())
}

fn insert_fixture(name: &str) -> anyhow::Result<Fixture> {
    let conn = db::open_db()?;
    let outcome = db::record_captured_event(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: &format!("{name}-{RAW_SENTINEL}"),
            project: &format!("{name}-project"),
            cwd: None,
            event_type: "tool_result",
            role: Some("tool"),
            tool_name: Some("Edit"),
            content: &format!("{RAW_SENTINEL} {SECRET_SENTINEL}"),
            task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
        },
    )?;
    let (session_id, project_id): (i64, i64) = conn.query_row(
        "SELECT session_row_id, project_id FROM captured_events WHERE id = ?1",
        params![outcome.event_row_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO observations
         (memory_session_id, project, type, title, narrative, created_at_epoch,
          status, project_id, session_row_id, observation_type, reference_time_epoch)
         VALUES (?1, ?2, 'discovery', 'Safe derived observation', ?3, NULL,
                 'active', ?4, ?5, 'discovery', ?6)",
        params![
            name,
            format!("{name}-project"),
            format!("{RAW_SENTINEL} {SECRET_SENTINEL}"),
            project_id,
            session_id,
            now
        ],
    )?;
    let observation_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workstreams
         (project, title, description, status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'Safe workstream', 'Safe derived progress', 'active', ?2, ?2)",
        params![format!("{name}-project"), now],
    )?;
    let workstream_id = conn.last_insert_rowid();
    let task_id: i64 = conn.query_row(
        "SELECT id FROM extraction_tasks WHERE project_id = ?1 ORDER BY id DESC LIMIT 1",
        params![project_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "UPDATE extraction_tasks
         SET last_error = ?2, failure_class = 'transient', status = 'failed'
         WHERE id = ?1",
        params![task_id, format!("{RAW_SENTINEL} {SECRET_SENTINEL}")],
    )?;
    Ok(Fixture {
        observation_id,
        session_id,
        workstream_id,
        event_id: outcome.event_row_id,
        task_id,
    })
}

fn insert_workstreams(conn: &Connection, prefix: &str, count: usize) -> anyhow::Result<Vec<i64>> {
    let now = chrono::Utc::now().timestamp();
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        conn.execute(
            "INSERT INTO workstreams
             (project, title, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, 'active', ?3, ?3)",
            params![
                format!("{prefix}-project"),
                format!("{prefix}-{index}"),
                now
            ],
        )?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

#[allow(clippy::too_many_arguments)]
fn insert_cursor_rows(
    conn: &Connection,
    kind: &str,
    prefix: &str,
    count: usize,
    host_id: i64,
    workspace_id: i64,
    project_id: i64,
    session_row_id: i64,
    raw_session_id: &str,
) -> anyhow::Result<Vec<i64>> {
    let now = chrono::Utc::now().timestamp();
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        match kind {
            "observations" => conn.execute(
                "INSERT INTO observations
                 (memory_session_id, project, type, title, created_at_epoch, status,
                  project_id, session_row_id, observation_type)
                 VALUES (?1, 'reinsert-base-project', 'discovery', ?2, NULL,
                         'active', ?3, ?4, 'discovery')",
                params![
                    format!("{prefix}-observation-{index}"),
                    format!("{prefix}-{index}"),
                    project_id,
                    session_row_id
                ],
            )?,
            "sessions" => conn.execute(
                "INSERT INTO sessions
                 (host_id, workspace_id, project_id, session_id, started_at_epoch,
                  last_seen_at_epoch, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'active')",
                params![
                    host_id,
                    workspace_id,
                    project_id,
                    format!("{prefix}-session-{index}"),
                    now
                ],
            )?,
            "workstreams" => conn.execute(
                "INSERT INTO workstreams
                 (project, title, status, created_at_epoch, updated_at_epoch)
                 VALUES ('reinsert-base-project', ?1, 'active', ?2, ?2)",
                params![format!("{prefix}-workstream-{index}"), now],
            )?,
            "events" => conn.execute(
                "INSERT INTO captured_events
                 (host_id, workspace_id, project_id, session_row_id, session_id,
                  event_id, event_type, role, tool_name, content_text, content_hash,
                  token_estimate, retention_class, created_at_epoch, inserted_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'tool_result', 'tool', 'Edit',
                         ?7, ?8, 1, 'inline', ?9, ?9)",
                params![
                    host_id,
                    workspace_id,
                    project_id,
                    session_row_id,
                    raw_session_id,
                    format!("{prefix}-event-{index}"),
                    RAW_SENTINEL,
                    format!("hash-{prefix}-{index}"),
                    now
                ],
            )?,
            "tasks" => conn.execute(
                "INSERT INTO extraction_tasks
                 (task_kind, host_id, workspace_id, project_id, session_row_id,
                  priority, status, idempotency_key, attempts, created_at_epoch,
                  updated_at_epoch)
                 VALUES ('observation_extract', ?1, ?2, ?3, ?4, 20, 'pending',
                         ?5, 0, ?6, ?6)",
                params![
                    host_id,
                    workspace_id,
                    project_id,
                    session_row_id,
                    format!("{prefix}-task-{index}"),
                    now
                ],
            )?,
            _ => anyhow::bail!("unknown cursor fixture kind"),
        };
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

fn delete_cursor_row(conn: &Connection, kind: &str, id: i64) -> anyhow::Result<()> {
    match kind {
        "observations" => conn.execute("DELETE FROM observations WHERE id = ?1", params![id])?,
        "sessions" => conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?,
        "workstreams" => conn.execute("DELETE FROM workstreams WHERE id = ?1", params![id])?,
        "events" => conn.execute("DELETE FROM captured_events WHERE id = ?1", params![id])?,
        "tasks" => conn.execute("DELETE FROM extraction_tasks WHERE id = ?1", params![id])?,
        _ => anyhow::bail!("unknown cursor fixture kind"),
    };
    Ok(())
}

async fn get_json(app: &Router, uri: &str, token: &str) -> anyhow::Result<(StatusCode, Value)> {
    let response = app
        .clone()
        .oneshot(authorized_request(Method::GET, uri, token, Body::empty()))
        .await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

fn ids(payload: &Value) -> Vec<i64> {
    payload["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_i64())
        .collect()
}

fn assert_safe_json(payload: &Value) {
    let serialized = payload.to_string();
    assert!(!serialized.contains(RAW_SENTINEL));
    assert!(!serialized.contains(SECRET_SENTINEL));
    assert!(!serialized.contains("content_text"));
    assert!(!serialized.contains("last_error"));
    assert!(!serialized.contains("idempotency_key"));
}

fn list_routes() -> [&'static str; 5] {
    [
        "/api/v1/observations",
        "/api/v1/sessions",
        "/api/v1/workstreams",
        "/api/v1/events",
        "/api/v1/tasks",
    ]
}

fn list_and_detail_routes(id: i64) -> Vec<String> {
    list_routes()
        .into_iter()
        .flat_map(|route| [route.to_string(), format!("{route}/{id}")])
        .collect()
}
