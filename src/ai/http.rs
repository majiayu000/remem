use std::sync::OnceLock;

use anyhow::{Context, Result};

use crate::ai::config::resolve_model_for_api;
use crate::ai::types::{AiCallResult, TokenUsage, AI_TIMEOUT_SECS};
use crate::runtime_config::ResolvedMemoryAiProfile;

/// Process-wide HTTP client shared across AI calls.
///
/// `reqwest::Client` owns a connection pool and is designed to be reused; a
/// fresh client per call threw away keep-alive connections and rebuilt the TLS
/// configuration every time. The timeout is a compile-time constant, so one
/// client serves every call. The client is cheap to reuse across base URLs
/// because the pool is keyed by host.
fn shared_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(AI_TIMEOUT_SECS))
        .build()
        .context("build shared AI HTTP client")?;
    // A concurrent caller may win the race; `set` fails in that case and we
    // return whichever client is now stored. Either way the client is built at
    // most a handful of times, then reused for the process lifetime.
    let _ = CLIENT.set(client);
    Ok(CLIENT
        .get()
        .expect("shared HTTP client was just initialized"))
}

pub(super) async fn call_http(
    system: &str,
    user_message: &str,
    profile: &ResolvedMemoryAiProfile,
) -> Result<AiCallResult> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_AUTH_TOKEN"))
        .context("ANTHROPIC_API_KEY not set")?;
    let raw = profile.model.as_deref().unwrap_or("haiku");
    let model = resolve_model_for_api(raw);
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": [{"type": "text", "text": system}],
        "messages": [{"role": "user", "content": user_message}]
    });

    let client = shared_client()?;

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
    let text = extract_text(&data)?;
    let usage = extract_usage(&data);

    Ok(AiCallResult {
        text,
        executor: "http",
        model: model.to_string(),
        usage,
        usage_source: Some("anthropic_usage"),
    })
}

fn extract_text(data: &serde_json::Value) -> Result<String> {
    let text = data["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|content| content["text"].as_str())
        .ok_or_else(|| {
            let snippet: String = serde_json::to_string(data)
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect();
            anyhow::anyhow!("Anthropic response missing content[0].text: {}", snippet)
        })?
        .to_string();

    if text.trim().is_empty() {
        anyhow::bail!("Anthropic returned empty text body");
    }
    Ok(text)
}

fn json_i64(data: &serde_json::Value, key: &str) -> i64 {
    data.get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
}

fn extract_usage(data: &serde_json::Value) -> Option<TokenUsage> {
    let usage = data.get("usage")?;
    let input_tokens = json_i64(usage, "input_tokens");
    let output_tokens = json_i64(usage, "output_tokens");
    let cache_creation_tokens = json_i64(usage, "cache_creation_input_tokens");
    let cache_read_tokens = json_i64(usage, "cache_read_input_tokens");
    let token_usage = TokenUsage {
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
        raw_input_tokens: input_tokens + cache_creation_tokens + cache_read_tokens,
        raw_output_tokens: output_tokens,
        ..TokenUsage::default()
    };
    (!token_usage.is_empty()).then_some(token_usage)
}

#[cfg(test)]
mod http_tests {
    use super::{extract_text, extract_usage, shared_client};
    use serde_json::json;

    #[test]
    fn shared_client_is_reused_across_calls() {
        let first = shared_client().expect("client builds");
        let second = shared_client().expect("client builds");
        assert!(
            std::ptr::eq(first, second),
            "shared_client must return the same process-wide client instance"
        );
    }

    #[test]
    fn extracts_text_from_valid_response() {
        let data = json!({
            "content": [{"type": "text", "text": "hello"}]
        });
        assert_eq!(extract_text(&data).unwrap(), "hello");
    }

    #[test]
    fn errors_on_tool_use_response_without_text_field() {
        let data = json!({
            "content": [{"type": "tool_use", "id": "abc", "name": "x", "input": {}}]
        });
        let err = extract_text(&data).unwrap_err().to_string();
        assert!(err.contains("missing content[0].text"), "got: {err}");
    }

    #[test]
    fn errors_on_missing_content_array() {
        let data = json!({"id": "msg_1"});
        let err = extract_text(&data).unwrap_err().to_string();
        assert!(err.contains("missing content[0].text"), "got: {err}");
    }

    #[test]
    fn errors_on_empty_content_array() {
        let data = json!({"content": []});
        assert!(extract_text(&data).is_err());
    }

    #[test]
    fn errors_on_whitespace_only_text() {
        let data = json!({"content": [{"type": "text", "text": "   \n"}]});
        let err = extract_text(&data).unwrap_err().to_string();
        assert!(err.contains("empty text body"), "got: {err}");
    }

    #[test]
    fn errors_on_empty_string_text() {
        let data = json!({"content": [{"type": "text", "text": ""}]});
        let err = extract_text(&data).unwrap_err().to_string();
        assert!(err.contains("empty text body"), "got: {err}");
    }

    #[test]
    fn extracts_anthropic_usage_breakdown() {
        let data = json!({
            "usage": {
                "input_tokens": 100,
                "output_tokens": 40,
                "cache_creation_input_tokens": 20,
                "cache_read_input_tokens": 300
            }
        });
        let usage = extract_usage(&data).expect("usage should parse");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.cache_creation_tokens, 20);
        assert_eq!(usage.cache_read_tokens, 300);
        assert_eq!(usage.raw_input_tokens, 420);
        assert_eq!(usage.total_tokens(), 460);
    }
}
