use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorMode {
    Http,
    Sdk,
    Composite,
}

impl ExecutorMode {
    pub fn from_env() -> Self {
        match std::env::var("CM_EXECUTOR_MODE").ok().as_deref() {
            Some("sdk") => Self::Sdk,
            Some("composite") => Self::Composite,
            _ => Self::Http,
        }
    }
}

fn get_api_key() -> Result<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_AUTH_TOKEN"))
        .context("ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN not set")
}

fn get_model() -> String {
    std::env::var("CLAUDE_MEM_MODEL")
        .unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string())
}

fn get_api_url() -> String {
    std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string())
}

/// Main entry point for AI calls. Routes to HTTP or SDK based on CM_EXECUTOR_MODE.
pub async fn call_ai(system: &str, user_message: &str) -> Result<String> {
    match ExecutorMode::from_env() {
        ExecutorMode::Http => call_http(system, user_message).await,
        #[cfg(feature = "sdk")]
        ExecutorMode::Sdk => call_sdk(system, user_message).await,
        #[cfg(not(feature = "sdk"))]
        ExecutorMode::Sdk => {
            tracing::warn!("SDK mode requested but 'sdk' feature not enabled; falling back to HTTP");
            call_http(system, user_message).await
        }
        #[cfg(feature = "sdk")]
        ExecutorMode::Composite => match call_sdk(system, user_message).await {
            Ok(text) => Ok(text),
            Err(e) => {
                tracing::warn!("SDK call failed, falling back to HTTP: {e}");
                call_http(system, user_message).await
            }
        },
        #[cfg(not(feature = "sdk"))]
        ExecutorMode::Composite => {
            tracing::warn!("Composite mode requested but 'sdk' feature not enabled; falling back to HTTP");
            call_http(system, user_message).await
        }
    }
}

async fn call_http(system: &str, user_message: &str) -> Result<String> {
    let api_key = get_api_key()?;
    let model = get_model();
    let base_url = get_api_url();

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": [{"type": "text", "text": system}],
        "messages": [{"role": "user", "content": user_message}]
    });

    let client = reqwest::Client::new();
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
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API error {}: {}", status, text);
    }

    let data: serde_json::Value = resp.json().await?;
    let text = data["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["text"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(text)
}

#[cfg(feature = "sdk")]
async fn call_sdk(system: &str, user_message: &str) -> Result<String> {
    use anthropic_agent_sdk::types::options::ClaudeAgentOptions;
    use futures::StreamExt;

    let model = get_model();
    let timeout: u64 = std::env::var("CM_SDK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120)
        .clamp(5, 600);

    let mut opts = ClaudeAgentOptions::default();
    opts.model = Some(model);
    opts.system_prompt = Some(anthropic_agent_sdk::SystemPrompt::String(system.to_string()));
    opts.max_turns = Some(1);
    opts.read_timeout_secs = Some(timeout);

    if let Ok(path) = std::env::var("CM_CLAUDE_CODE_PATH") {
        if !path.trim().is_empty() {
            opts.path_to_claude_code_executable = Some(std::path::PathBuf::from(path));
        }
    }

    if let Ok(cwd) = std::env::var("CM_SDK_OBSERVER_CWD") {
        if !cwd.trim().is_empty() {
            opts.cwd = Some(std::path::PathBuf::from(cwd));
        }
    }

    let stream = anthropic_agent_sdk::query::query(user_message, Some(opts))
        .await
        .context("SDK query failed")?;

    tokio::pin!(stream);

    let mut text_parts: Vec<String> = Vec::new();

    while let Some(result) = stream.next().await {
        let message = result.context("SDK stream error")?;

        if let anthropic_agent_sdk::types::messages::Message::Assistant {
            message: content, ..
        } = message
        {
            for block in &content.content {
                if let anthropic_agent_sdk::types::messages::ContentBlock::Text { text } = block {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        text_parts.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    let full_text = text_parts.join("\n");
    if full_text.is_empty() {
        anyhow::bail!("SDK returned empty response");
    }

    Ok(full_text)
}
