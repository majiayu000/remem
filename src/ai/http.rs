use anyhow::{Context, Result};

use crate::ai::config::{get_model_raw, resolve_model_for_api};
use crate::ai::types::{AiCallResult, AI_TIMEOUT_SECS};

pub(super) async fn call_http(system: &str, user_message: &str) -> Result<AiCallResult> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_AUTH_TOKEN"))
        .context("ANTHROPIC_API_KEY not set")?;
    let raw = get_model_raw();
    let model = resolve_model_for_api(&raw);
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": [{"type": "text", "text": system}],
        "messages": [{"role": "user", "content": user_message}]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(AI_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .post(format!("{}/v1/messages", base_url.trim_end_matches('/')))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .unwrap_or_else(|error| format!("<body read error: {}>", error));
        anyhow::bail!("Anthropic API error {}: {}", status, text);
    }

    let data: serde_json::Value = resp.json().await?;
    let text = data["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|content| content["text"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(AiCallResult {
        text,
        executor: "http",
        model: model.to_string(),
    })
}
