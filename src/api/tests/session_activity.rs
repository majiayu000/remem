use super::*;

#[tokio::test]
async fn activity_rest_labels_redact_projected_topics_and_preserve_intent_filter(
) -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-activity-label-redaction");
    let conn = db::open_db()?;
    let capture = db::record_captured_event(
        &conn,
        &db::CaptureEventInput {
            host: "codex-cli",
            session_id: "label-read",
            project: "/label-read",
            cwd: None,
            event_type: "message",
            role: Some("user"),
            tool_name: None,
            content: "repair labels",
            task_kind: None,
        },
    )?;
    let session_row_id: i64 = conn.query_row(
        "SELECT session_row_id FROM captured_events WHERE id = ?1",
        [capture.event_row_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO raw_session_identities
         (source_root, transcript_path, host, fallback_session_id, canonical_session_id,
          project, legacy_project, status, observed_mtime_ns, observed_size_bytes,
          first_seen_at_epoch, last_seen_at_epoch)
         VALUES ('local', '/tmp/label-read.jsonl', 'codex-cli', 'label-read', 'label-read',
                 '/label-read', 'label-read', 'active', 1, 1, 1, 1)",
        [],
    )?;
    conn.execute(
        "INSERT INTO raw_messages
         (session_id, project, role, content, content_hash, source, created_at_epoch,
          source_root, transcript_identity_id, transcript_record_ordinal)
         VALUES ('label-read', '/label-read', 'user', 'repair labels', 'label-read-hash',
                 'transcript', 1735660800, 'local', ?1, 1)",
        [conn.last_insert_rowid()],
    )?;
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, session_row_id, created_at_epoch,
          session_intent, session_topic, session_intent_source)
         VALUES ('label-read', '/label-read', ?1, 1735660800, 'fix', 'Safe topic', 'summary')",
        [session_row_id],
    )?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = crate::api::build_router(0).with_state(DbState);
    for (topic, expected) in [
        (
            "Investigate token=abc123".to_string(),
            "Investigate token=[REDACTED]".to_string(),
        ),
        (
            "curl --oauth2-bearer tiny-token".to_string(),
            "curl --oauth2-bearer [REDACTED]".to_string(),
        ),
        (
            "--oauth2-bearer tiny-token".to_string(),
            "--oauth2-bearer [REDACTED]".to_string(),
        ),
        ("-u alice:pw".to_string(), "-u [REDACTED]".to_string()),
        (
            format!("{} token=x", ("Review label ".repeat(5) + "results")),
            format!(
                "{} token=[REDACTED]",
                ("Review label ".repeat(5) + "results")
            ),
        ),
    ] {
        conn.execute("UPDATE session_summaries SET session_topic = ?1", [&topic])?;
        for filter in ["fix", "abstain"] {
            let response = app
                .clone()
                .oneshot(authorized_request(
                    Method::GET,
                    &format!("/api/v1/session-activity/sessions?session_intent={filter}"),
                    &token,
                    Body::empty(),
                ))
                .await?;
            assert_eq!(response.status(), StatusCode::OK);
            let payload: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
            if filter == "abstain" {
                assert_eq!(payload["data"], serde_json::json!([]));
                continue;
            }
            assert_eq!(payload["data"].as_array().unwrap().len(), 1);
            assert_eq!(payload["data"][0]["session_topic"], expected);
            assert_eq!(
                payload["data"][0]["display_label"],
                format!("0101｜fix｜{expected}")
            );
            let encoded = payload.to_string();
            for secret in ["abc123", "tiny-token", "token=x", "alice:pw"] {
                assert!(!encoded.contains(secret), "{encoded}");
            }
        }
    }
    Ok(())
}

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
    assert!(sessions["data"][0]["session_row_id"].is_null());
    assert_eq!(sessions["data"][0]["override_available"], false);
    assert_eq!(sessions["data"][0]["mmdd"], "0101");
    assert!(sessions["data"][0]["display_label"].is_null());

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
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-stats?project=activity%2Fproject&since_epoch=90&until_epoch=200",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(stats.status(), StatusCode::OK);
    let stats: Value = serde_json::from_slice(&to_bytes(stats.into_body(), usize::MAX).await?)?;
    assert_eq!(stats["data"]["sessions"], 1);
    assert_eq!(stats["data"]["turns"], 1);
    assert_eq!(stats["data"]["actions"], 0);
    let conn = db::open_db()?;
    conn.execute("INSERT INTO memory_suppressions(target_kind,target_value,reason,actor,status,created_at_epoch,updated_at_epoch) VALUES ('pattern','activity/project','test','test','active',1,1)", [])?;
    drop(conn);
    let hidden = app
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-activity/sessions?session_intent=abstain",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(hidden.status(), StatusCode::OK);
    let hidden: Value = serde_json::from_slice(&to_bytes(hidden.into_body(), usize::MAX).await?)?;
    assert_eq!(hidden["data"], serde_json::json!([]));
    Ok(())
}

#[tokio::test]
async fn session_activity_rejects_invalid_windows_and_ids() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-session-activity-invalid");
    db::open_db()?;
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = crate::api::build_router(0).with_state(DbState);

    for query in [
        "session_intent=bugfix",
        "since_epoch=20&until_epoch=10",
        "since_epoch=20&until_epoch=20",
    ] {
        let response = app
            .clone()
            .oneshot(authorized_request(
                Method::GET,
                &format!("/api/v1/session-activity/sessions?{query}"),
                &token,
                Body::empty(),
            ))
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
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
        .clone()
        .oneshot(authorized_request(
            Method::GET,
            "/api/v1/session-activity/sessions?cursor=not-a-cursor",
            &token,
            Body::empty(),
        ))
        .await?;
    assert_eq!(id.status(), StatusCode::BAD_REQUEST);

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
