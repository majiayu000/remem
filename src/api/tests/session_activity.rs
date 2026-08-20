use super::*;

#[tokio::test]
async fn session_activity_routes_project_list_detail_and_report_stats() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-session-activity");
    let conn = db::open_db()?;
    conn.execute(
        "INSERT INTO raw_messages
         (id, session_id, project, role, content, content_hash, source, cwd,
          created_at_epoch, source_root, event_time_source)
         VALUES
         (901, 'activity-session', 'activity/project', 'user',
          'Add an evidence-first session view', 'activity-user', 'transcript',
          '/activity', 100, 'local', 'transcript_event'),
         (902, 'activity-session', 'activity/project', 'assistant',
          'The session activity view is implemented and verified.',
          'activity-assistant', 'transcript', '/activity', 120, 'local',
          'transcript_event')",
        [],
    )?;
    drop(conn);

    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = crate::api::build_router(0).with_state(DbState);
    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/session-stats")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let projected = app
        .clone()
        .oneshot(authorized_json_request(
            Method::POST,
            "/api/v1/session-activity/project",
            &token,
            r#"{"source_root":"local","project":"activity/project","session_id":"activity-session"}"#,
        ))
        .await?;
    assert_eq!(projected.status(), StatusCode::OK);
    let projected: Value =
        serde_json::from_slice(&to_bytes(projected.into_body(), usize::MAX).await?)?;
    assert_eq!(projected["data"]["changed"], true);
    assert_eq!(projected["data"]["turn_count"], 1);

    let sessions = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-activity/sessions?project=activity%2Fproject",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(sessions.status(), StatusCode::OK);
    let sessions: Value =
        serde_json::from_slice(&to_bytes(sessions.into_body(), usize::MAX).await?)?;
    assert_eq!(sessions["data"][0]["projected_turn_count"], 1);

    let turns = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-activity?session_id=activity-session",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(turns.status(), StatusCode::OK);
    let turns: Value = serde_json::from_slice(&to_bytes(turns.into_body(), usize::MAX).await?)?;
    assert_eq!(
        turns["data"][0]["user_said"],
        "Add an evidence-first session view"
    );
    assert_eq!(turns["data"][0]["capture_health"], "unavailable");
    let id = turns["data"][0]["id"].as_i64().expect("turn id");

    let detail = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            &format!("/api/v1/session-activity/{id}"),
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(detail.status(), StatusCode::OK);

    let stats = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-stats?project=activity%2Fproject",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(stats.status(), StatusCode::OK);
    let stats: Value = serde_json::from_slice(&to_bytes(stats.into_body(), usize::MAX).await?)?;
    assert_eq!(stats["data"]["sessions"], 1);
    assert_eq!(stats["data"]["turns"], 1);
    assert_eq!(stats["data"]["actions"], 0);
    Ok(())
}

#[tokio::test]
async fn session_activity_rejects_invalid_windows_and_ids() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-session-activity-invalid");
    db::open_db()?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = crate::api::build_router(0).with_state(DbState);

    let window = app
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-stats?since_epoch=20&until_epoch=10",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(window.status(), StatusCode::BAD_REQUEST);

    let id = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-activity/not-an-id",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(id.status(), StatusCode::BAD_REQUEST);
    Ok(())
}
