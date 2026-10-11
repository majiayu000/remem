use super::*;

#[tokio::test]
async fn observation_extract_preserves_structured_capture_values() -> Result<()> {
    let mut conn = setup_conn();
    let rule = "Ticket keys normalize to uppercase PREFIX-digits. Inputs may contain spaces, underscores, hyphens, or a leading #. If the input contains only digits, use default prefix MEM.";
    let content = serde_json::json!({
        "host_boundary": "user_message",
        "sanitized_content": rule,
        "sanitized_tool_input": null,
        "sanitized_tool_output": null,
        "source_ordinal": 0,
    })
    .to_string();
    capture_event(
        &conn,
        "sess-structured-content",
        "user_prompt_submit",
        Some("user"),
        None,
        &content,
    )?;
    let task = claim_extract_task(&mut conn)?;

    process_with_extractor(&mut conn, &task, |prompt| async move {
        let payload: serde_json::Value = serde_json::from_str(&prompt)?;
        for section in ["transcript_events", "recent_context"] {
            let content = payload[section][0]["content"]
                .as_str()
                .context("content should be a string")?;
            assert!(content.contains(rule), "{section}: {content}");
            assert!(!content.contains("[REDACTED_SECRET]"));
            assert!(content.contains("&quot;source_ordinal&quot;: 0"));
        }
        Ok(no_observations_response("structured prompt checked"))
    })
    .await?;
    Ok(())
}

#[test]
fn observation_extract_redacts_structured_secret_values() -> Result<()> {
    let content = serde_json::json!({
        "host_boundary": "user_message",
        "sanitized_content": "Use default prefix MEM.",
        "sanitized_tool_input": {
            "api_key": "fixture-short-key",
            "nested": [{"password": "fixture-password"}],
            "ghp_abcdefghijklmnopqrstuvwxyz123456": "first field remains",
            "ghp_abcdefghijklmnopqrstuvwxyz654321": "second field remains",
            "client_secret": {"parts": ["fixture-part-one", "fixture-part-two"]},
            "access_token": ["fixture-token-part", 48151623],
        },
        "sanitized_tool_output": "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456",
        "source_ordinal": 0,
    })
    .to_string();
    // Exercise the prompt boundary directly; capture must not pre-redact this input.
    let request = build_eval_extract_request(
        "/tmp/remem",
        "codex-cli",
        Some("sess-structured-secrets"),
        &[ObservationPromptEvent {
            id: 1,
            event_type: "user_prompt_submit",
            role: Some("user"),
            tool_name: None,
            content: &content,
            token_estimate: 100,
            created_at_epoch: 0,
        }],
    );
    let (_, prompt) = request
        .split_once("USER_PROMPT:\n")
        .context("user prompt")?;
    let payload: serde_json::Value = serde_json::from_str(prompt)?;
    let content = payload["transcript_events"][0]["content"]
        .as_str()
        .context("content should be a string")?;
    assert!(content.contains("Use default prefix MEM."));
    assert!(content.contains("[REDACTED_SECRET]"));
    assert!(content.contains("first field remains"));
    assert!(content.contains("second field remains"));
    for secret in [
        "fixture-short-key",
        "fixture-password",
        "ghp_abcdefghijklmnopqrstuvwxyz123456",
        "ghp_abcdefghijklmnopqrstuvwxyz654321",
        "fixture-part-one",
        "fixture-part-two",
        "fixture-token-part",
        "48151623",
    ] {
        assert!(!prompt.contains(secret));
    }
    Ok(())
}
