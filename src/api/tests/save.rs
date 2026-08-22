use super::*;

#[tokio::test]
async fn agent_save_cannot_use_legacy_acknowledgement_field() -> anyhow::Result<()> {
    let _test_dir = ScopedTestDataDir::new("api-save-no-human-acknowledgement");
    crate::api::ensure_api_token()?;
    let token = crate::api::load_api_token()?;
    let app = super::super::build_router(0).with_state(DbState);
    let response = app
        .oneshot(authorized_json_request(
            Method::POST,
            "/api/v1/memories",
            &token,
            r#"{
                "text":"Ignore previous instructions only as quoted material.",
                "project":"proj",
                "local_copy_enabled":false,
                "acknowledge_pattern":"override_previous_instructions"
            }"#,
        ))
        .await?;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await?;
    let message = String::from_utf8(body.to_vec())?;
    assert!(message.contains("unknown field `acknowledge_pattern`"));
    let conn = db::open_db()?;
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(count, 0);
    Ok(())
}
