use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::db::{self, test_support::ScopedTestDataDir};

async fn endpoint(method: Method, path: &str, body: Value) -> (StatusCode, Value) {
    crate::api::ensure_api_token().unwrap();
    let token = crate::api::load_api_token().unwrap();
    let response = crate::api::build_router(0)
        .with_state(crate::api::DbState)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

fn session(conn: &Connection) -> i64 {
    let capture = db::record_captured_event(
        conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "eligible-intent-test",
            project: "test",
            cwd: None,
            event_type: "tool_result",
            role: Some("tool"),
            tool_name: Some("Edit"),
            content: "eligible intent fixture",
            task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
        },
    )
    .unwrap();
    conn.query_row(
        "SELECT session_row_id FROM captured_events WHERE id=?1",
        [capture.event_row_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn summary(conn: &Connection, session: i64, topic: &str, epoch: i64, status: &str) -> i64 {
    conn.execute(
        "INSERT INTO session_summaries(memory_session_id,project,session_row_id,
         created_at_epoch,session_intent,session_topic,session_intent_source,poisoning_status)
         VALUES (?1,'test',?2,?3,'fix',?1,'summary',?4)",
        params![topic, session, epoch, status],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn correction(session: i64) -> Value {
    json!({"targets":[{"kind":"session","id":session}],
        "session_intent":"doc","session_topic":"Corrected topic","reason":"Correct classification"})
}

async fn preview(session: i64) -> (StatusCode, Value) {
    endpoint(
        Method::POST,
        "/api/v1/session-intent/preview",
        correction(session),
    )
    .await
}

async fn apply(preview: &Value) -> (StatusCode, Value) {
    endpoint(
        Method::POST,
        "/api/v1/session-intent/apply",
        json!({"preview_token":preview["preview_token"],"confirm":true}),
    )
    .await
}

async fn listed() -> Value {
    let (status, value) = endpoint(Method::GET, "/api/v1/sessions", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"].as_array().unwrap().len(), 1);
    value["data"][0].clone()
}

#[tokio::test]
async fn session_intent_override_targets_listed_eligible_summary() {
    let _scope = ScopedTestDataDir::new("intent-eligible-apply");
    let conn = db::open_db().unwrap();
    let session = session(&conn);
    let safe = summary(&conn, session, "Visible safe topic", 1, "safe");
    let hidden = summary(&conn, session, "Hidden quarantined topic", 2, "quarantined");
    let list = listed().await;
    assert_eq!(list["session_topic"], "Visible safe topic");
    let (status, value) = preview(session).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        value["changes"][0]["before"]["session_topic"],
        list["session_topic"]
    );
    assert_eq!(
        value["changes"][0]["before"]["session_intent"],
        list["session_intent"]
    );

    // A hidden summary changing must not invalidate a preview of the visible label.
    conn.execute(
        "UPDATE session_summaries SET session_topic='Hidden changed topic',
         session_intent_updated_at_epoch=3 WHERE id=?1",
        [hidden],
    )
    .unwrap();
    let hidden_before: String = conn
        .query_row(
            "SELECT json_object('intent',session_intent,'topic',session_topic,
         'source',session_intent_source,'epoch',session_intent_updated_at_epoch,
         'status',poisoning_status) FROM session_summaries WHERE id=?1",
            [hidden],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(apply(&value).await.0, StatusCode::OK);
    let hidden_after: String = conn
        .query_row(
            "SELECT json_object('intent',session_intent,'topic',session_topic,
         'source',session_intent_source,'epoch',session_intent_updated_at_epoch,
         'status',poisoning_status) FROM session_summaries WHERE id=?1",
            [hidden],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hidden_after, hidden_before);
    let changed: (String, String, String) = conn.query_row(
        "SELECT session_intent,session_topic,session_intent_source FROM session_summaries WHERE id=?1",
        [safe], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(
        changed,
        ("doc".into(), "Corrected topic".into(), "override".into())
    );
    assert_eq!(listed().await["session_topic"], "Corrected topic");
}

#[tokio::test]
async fn session_intent_override_eligible_fingerprint_tracks_fields_and_row_identity() {
    let _scope = ScopedTestDataDir::new("intent-eligible-stale");
    let conn = db::open_db().unwrap();
    let session = session(&conn);
    let safe = summary(&conn, session, "Visible safe topic", 1, "safe");
    summary(&conn, session, "Hidden quarantined topic", 2, "quarantined");
    let (status, value) = preview(session).await;
    assert_eq!(status, StatusCode::OK);
    conn.execute(
        "UPDATE session_summaries SET session_topic='Visible changed topic' WHERE id=?1",
        [safe],
    )
    .unwrap();
    let (status, error) = apply(&value).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "preview_stale");

    let (status, value) = preview(session).await;
    assert_eq!(status, StatusCode::OK);
    // All fingerprinted fields are identical except the new eligible row's id.
    let replacement = summary(&conn, session, "Visible changed topic", 1, "safe");
    let (status, error) = apply(&value).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "preview_stale");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type='session_intent_override'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT session_intent_source FROM session_summaries WHERE id=?1",
            [replacement],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "summary"
    );
}

#[tokio::test]
async fn session_intent_override_no_eligible_summary_and_quarantined_override_controls() {
    let _scope = ScopedTestDataDir::new("intent-eligible-controls");
    let conn = db::open_db().unwrap();
    let session = session(&conn);
    let hidden = summary(&conn, session, "Hidden quarantined topic", 2, "quarantined");
    assert!(listed().await["session_topic"].is_null());
    let (status, error) = preview(session).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "session_summary_required");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type='session_intent_preview'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );

    conn.execute(
        "UPDATE session_summaries SET session_intent_source='override' WHERE id=?1",
        [hidden],
    )
    .unwrap();
    let list = listed().await;
    let (status, value) = preview(session).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        value["changes"][0]["before"]["session_topic"],
        list["session_topic"]
    );
    assert_eq!(apply(&value).await.0, StatusCode::OK);
    assert_eq!(
        conn.query_row(
            "SELECT poisoning_status FROM session_summaries WHERE id=?1",
            [hidden],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "quarantined"
    );
}

#[tokio::test]
async fn session_intent_override_eligible_order_uses_intent_epoch_and_id() {
    let _scope = ScopedTestDataDir::new("intent-eligible-order");
    let conn = db::open_db().unwrap();
    let session = session(&conn);
    let selected = summary(&conn, session, "Older updated topic", 1, "safe");
    summary(&conn, session, "Newer created topic", 2, "safe");
    summary(&conn, session, "Hidden newest topic", 4, "quarantined");
    conn.execute(
        "UPDATE session_summaries SET session_intent_updated_at_epoch=3 WHERE id=?1",
        [selected],
    )
    .unwrap();
    let list = listed().await;
    assert_eq!(list["session_topic"], "Older updated topic");
    let (status, value) = preview(session).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        value["changes"][0]["before"]["session_topic"],
        list["session_topic"]
    );
    assert_eq!(apply(&value).await.0, StatusCode::OK);
    assert_eq!(listed().await["session_topic"], "Corrected topic");
}
